package bluemaprs.paper.ipc;

import java.io.IOException;
import java.util.ArrayDeque;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.locks.Condition;
import java.util.concurrent.locks.ReentrantLock;

/**
 * Bounded writer queue. Ordered items are never dropped; latest-wins items ({@code Players}, {@code Markers} per map)
 * keep the queue position of their first unsent version, so a {@code Shutdown} queued later still goes out after them.
 */
final class OutboundQueue {

    private final ReentrantLock lock = new ReentrantLock();
    private final Condition notEmpty = lock.newCondition();
    private final Condition notFull = lock.newCondition();
    /** {@code List<Frame>} (ordered item) or {@code String} (key into {@link #latest}). */
    private final ArrayDeque<Object> order = new ArrayDeque<>();
    private final Map<String, List<Frame>> latest = new HashMap<>();
    private final int capacity;
    private boolean closed;

    OutboundQueue(int capacity) {
        this.capacity = capacity;
    }

    void put(List<Frame> frames, long timeoutMs) throws IOException {
        lock.lock();
        try {
            long nanos = TimeUnit.MILLISECONDS.toNanos(timeoutMs);
            while (!closed && order.size() >= capacity) {
                if (nanos <= 0) throw new IOException("IPC queue to the BlueMap core is full");
                nanos = notFull.awaitNanos(nanos);
            }
            if (closed) throw new IOException("IPC connection to the BlueMap core is closed");
            order.add(frames);
            notEmpty.signal();
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IOException("interrupted while queueing an IPC message", e);
        } finally {
            lock.unlock();
        }
    }

    void putLatest(String key, List<Frame> frames) {
        lock.lock();
        try {
            if (closed) return;
            if (latest.put(key, frames) == null) order.add(key);
            notEmpty.signal();
        } finally {
            lock.unlock();
        }
    }

    /** Rejects new items; {@link #take} drains what is queued and then returns {@code null}. */
    void close() {
        lock.lock();
        try {
            closed = true;
            notEmpty.signalAll();
            notFull.signalAll();
        } finally {
            lock.unlock();
        }
    }

    @SuppressWarnings("unchecked")
    List<Frame> take() throws InterruptedException {
        lock.lock();
        try {
            while (order.isEmpty()) {
                if (closed) return null;
                notEmpty.await();
            }
            Object item = order.poll();
            notFull.signal();
            return item instanceof String key ? latest.remove(key) : (List<Frame>) item;
        } finally {
            lock.unlock();
        }
    }

}
