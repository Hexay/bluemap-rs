//! `server-unpack` slots (docs/18 "Clients that don't unpack"): a bounded wait queue shared fairly by client.
//!
//! A free slot is taken at once. Otherwise the request waits up to `wait` in its client's FIFO; a freed slot is
//! handed straight to the oldest waiter of the next client in turn (round-robin), so arrivals can't barge past
//! waiters and a flooding client gets one slot in n, however many requests it queues. A client may fill the whole
//! queue while it is alone; once it is full, a client under its fair share (`queue / clients waiting`, at least
//! `min_share`) gets in by refusing the newest waiter of the client furthest over its share.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::Thread;
use std::time::{Duration, Instant};

use crate::client_key::ClientKey;

const WAITING: u8 = 0;
const GRANTED: u8 = 1;
const REFUSED: u8 = 2;

pub(crate) struct Slots {
    state: Mutex<State>,
    limit: usize,
    queue: usize,
    min_share: usize,
    wait: Duration,
}

#[derive(Default)]
struct State {
    busy: usize,
    waiting: usize,
    clients: HashMap<ClientKey, VecDeque<Arc<Waiter>>>,
    /// Clients with waiters, the next to get a slot first.
    turn: VecDeque<ClientKey>,
}

struct Waiter {
    thread: Thread,
    /// Changed only under the lock.
    outcome: AtomicU8,
}

pub(crate) struct SlotPermit<'a>(&'a Slots);

impl Slots {
    pub(crate) fn new(limit: usize, queue: usize, min_share: usize, wait: Duration) -> Self {
        Self { state: Mutex::default(), limit, queue, min_share, wait }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `None` at once when there is no room for `client` in the queue, or once its wait runs out.
    pub(crate) fn acquire(&self, client: ClientKey) -> Option<SlotPermit<'_>> {
        let me = {
            let mut state = self.lock();
            if state.busy < self.limit {
                state.busy += 1;
                return Some(SlotPermit(self));
            }
            if state.waiting >= self.queue && !self.make_room(&mut state, client) {
                return None;
            }
            let me = Arc::new(Waiter { thread: std::thread::current(), outcome: AtomicU8::new(WAITING) });
            let queue = state.clients.entry(client).or_default();
            queue.push_back(me.clone());
            let first = queue.len() == 1;
            if first {
                state.turn.push_back(client);
            }
            state.waiting += 1;
            me
        };
        let deadline = Instant::now().checked_add(self.wait);
        // park can wake spuriously (or from a stale unpark of this pooled thread): the outcome decides
        while me.outcome.load(Ordering::Acquire) == WAITING {
            match deadline.map(|d| d.saturating_duration_since(Instant::now())) {
                Some(Duration::ZERO) => break,
                Some(left) => std::thread::park_timeout(left),
                None => std::thread::park(),
            }
        }
        let mut state = self.lock();
        match me.outcome.load(Ordering::Acquire) {
            GRANTED => Some(SlotPermit(self)),
            REFUSED => None,
            _ => {
                state.remove(client, &me);
                None
            }
        }
    }

    /// The queue is full: refuses the newest waiter of the client furthest over its fair share, if `client` is
    /// under it. `true` when that freed a place.
    fn make_room(&self, state: &mut State, client: ClientKey) -> bool {
        let held = state.clients.get(&client).map_or(0, VecDeque::len);
        let clients = state.clients.len() + usize::from(held == 0);
        let share = (self.queue / clients).max(self.min_share);
        if held >= share {
            return false;
        }
        let Some((&over, _)) = state.clients.iter().filter(|(_, q)| q.len() > share).max_by_key(|(_, q)| q.len())
        else {
            return false;
        };
        let newest = state.clients.get_mut(&over).and_then(VecDeque::pop_back).expect("over its share");
        if state.clients[&over].is_empty() {
            state.forget(over);
        }
        state.waiting -= 1;
        newest.outcome.store(REFUSED, Ordering::Release);
        newest.thread.unpark();
        true
    }

    fn release(&self) {
        let next = {
            let mut state = self.lock();
            let next = state.next_waiter();
            match &next {
                Some(w) => w.outcome.store(GRANTED, Ordering::Release),
                None => state.busy -= 1,
            }
            next
        };
        if let Some(w) = next {
            w.thread.unpark();
        }
    }
}

impl State {
    /// The oldest waiter of the client whose turn it is; the client goes to the back of the turn.
    fn next_waiter(&mut self) -> Option<Arc<Waiter>> {
        let client = self.turn.pop_front()?;
        let queue = self.clients.get_mut(&client).expect("clients in turn have waiters");
        let waiter = queue.pop_front().expect("non-empty");
        if queue.is_empty() {
            self.clients.remove(&client);
        } else {
            self.turn.push_back(client);
        }
        self.waiting -= 1;
        Some(waiter)
    }

    /// A waiter that gave up.
    fn remove(&mut self, client: ClientKey, waiter: &Arc<Waiter>) {
        let Some(queue) = self.clients.get_mut(&client) else { return };
        let Some(at) = queue.iter().position(|w| Arc::ptr_eq(w, waiter)) else { return };
        queue.remove(at);
        if queue.is_empty() {
            self.forget(client);
        }
        self.waiting -= 1;
    }

    fn forget(&mut self, client: ClientKey) {
        self.clients.remove(&client);
        self.turn.retain(|c| *c != client);
    }
}

impl Drop for SlotPermit<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Mutex as StdMutex;

    use super::*;

    fn client(n: u8) -> ClientKey {
        ClientKey::of_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, n)))
    }

    fn waiting(slots: &Slots, n: usize) {
        while slots.lock().waiting != n {
            std::thread::yield_now();
        }
    }

    #[test]
    fn bounded_waits_queue_then_give_up() {
        let slots = Slots::new(1, 1, 1, Duration::from_millis(50));
        let held = slots.acquire(client(1)).expect("free");
        std::thread::scope(|s| {
            let waiter = s.spawn(|| slots.acquire(client(1)).is_some());
            waiting(&slots, 1);
            assert!(slots.acquire(client(1)).is_none(), "queue full: refused at once");
            drop(held);
            assert!(waiter.join().unwrap(), "a freed slot goes to the waiter");
        });
        let _held = slots.acquire(client(1)).unwrap();
        let t = Instant::now();
        assert!(slots.acquire(client(1)).is_none());
        assert!(t.elapsed() >= Duration::from_millis(50), "waited out the deadline");
        assert_eq!(slots.lock().waiting, 0);
        assert!(Slots::new(0, 0, 0, Duration::from_secs(9)).acquire(client(1)).is_none());
    }

    /// Slots go round-robin across clients, each client's waiters in arrival order.
    #[test]
    fn slots_go_round_robin_across_clients() {
        let slots = Slots::new(1, 16, 1, Duration::from_secs(30));
        let order = StdMutex::new(Vec::new());
        let held = slots.acquire(client(9)).unwrap();
        std::thread::scope(|s| {
            let mut queued = 0;
            for (who, n) in [(1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (3, 0)] {
                let (slots, order) = (&slots, &order);
                s.spawn(move || {
                    let _permit = slots.acquire(client(who)).unwrap();
                    order.lock().unwrap().push((who, n));
                });
                queued += 1;
                waiting(slots, queued);
            }
            drop(held);
        });
        assert_eq!(*order.lock().unwrap(), [(1, 0), (2, 0), (3, 0), (1, 1), (2, 1), (1, 2)]);
    }

    /// One client alone may fill the queue; a second one still gets its share, by refusing the first one's newest.
    #[test]
    fn a_full_queue_makes_room_for_a_client_under_its_share() {
        let slots = Slots::new(1, 4, 1, Duration::from_secs(30));
        let held = slots.acquire(client(9)).unwrap();
        std::thread::scope(|s| {
            let flood: Vec<_> = (0..4)
                .map(|i| {
                    let slots = &slots;
                    let w = s.spawn(move || slots.acquire(client(1)).is_some());
                    waiting(slots, i + 1);
                    w
                })
                .collect();
            assert!(slots.acquire(client(1)).is_none(), "alone and full: refused");
            let others: Vec<_> = (0..2)
                .map(|i| {
                    let slots = &slots;
                    let w = s.spawn(move || slots.acquire(client(2)).is_some());
                    // the refused flood waiter leaves at once, so the count stays at 4
                    while slots.lock().clients.get(&client(2)).map_or(0, VecDeque::len) != i + 1 {
                        std::thread::yield_now();
                    }
                    w
                })
                .collect();
            waiting(&slots, 4);
            assert!(slots.acquire(client(2)).is_none(), "at its share (4 / 2)");
            drop(held);
            let flood: Vec<bool> = flood.into_iter().map(|w| w.join().unwrap()).collect();
            assert_eq!(flood, [true, true, false, false], "the newest two were refused");
            assert!(others.into_iter().all(|w| w.join().unwrap()));
        });
        let state = slots.lock();
        assert_eq!((state.waiting, state.clients.len(), state.turn.len()), (0, 0, 0));
    }
}
