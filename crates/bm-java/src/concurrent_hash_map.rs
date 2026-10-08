//! Iteration order of a `java.util.concurrent.ConcurrentHashMap<String, _>` filled by one thread (JDK 25). Unlike
//! `HashMap`, a resize keeps only the longest same-half tail of each list bin in order and prepends the rest, a
//! long bin in a small table presizes the table 8×, and tree bins put new keys first.

use crate::string_hash;

const TREEIFY_THRESHOLD: usize = 8;
const UNTREEIFY_THRESHOLD: usize = 6;
const MIN_TREEIFY_CAPACITY: usize = 64;

/// (spread hash, key)
type Node<'a> = (u32, &'a str);

#[derive(Clone, Default)]
struct Bin<'a> {
    tree: bool,
    nodes: Vec<Node<'a>>,
}

/// Keys in the order a `new ConcurrentHashMap<>()` iterates them after `put`ting `keys` in the given order
/// (duplicates keep their first position). Since JDK 25 `putAll` into an empty map is the same sequence of puts.
pub fn concurrent_hash_map_order<S: AsRef<str>>(keys: &[S]) -> Vec<&str> {
    let mut table: Vec<Bin> = vec![Bin::default(); 16];
    let mut count = 0;
    for key in keys {
        let key = key.as_ref();
        let hash = spread(string_hash(key));
        let mask = table.len() - 1;
        let bin = &mut table[hash as usize & mask];
        if bin.nodes.iter().any(|&(_, k)| k == key) {
            continue;
        }
        let existing = bin.nodes.len();
        if bin.tree {
            bin.nodes.insert(0, (hash, key));
        } else {
            bin.nodes.push((hash, key));
            if existing >= TREEIFY_THRESHOLD {
                treeify_bin(&mut table, hash);
            }
        }
        count += 1;
        while count >= size_ctl(table.len()) {
            table = transfer(&table);
        }
    }
    table.into_iter().flat_map(|b| b.nodes).map(|(_, k)| k).collect()
}

fn spread(h: i32) -> u32 {
    let h = h as u32;
    (h ^ (h >> 16)) & 0x7fff_ffff
}

fn size_ctl(n: usize) -> usize {
    n - (n >> 2)
}

fn treeify_bin(table: &mut Vec<Bin>, hash: u32) {
    let n = table.len();
    if n < MIN_TREEIFY_CAPACITY {
        // tryPresize(n << 1): grows until sizeCtl >= tableSizeFor(3n + 1)
        let c = (3 * n + 1).next_power_of_two();
        while c > size_ctl(table.len()) {
            *table = transfer(table);
        }
    } else {
        table[hash as usize & (n - 1)].tree = true;
    }
}

fn transfer<'a>(table: &[Bin<'a>]) -> Vec<Bin<'a>> {
    let n = table.len();
    let high = |h: u32| h as usize & n != 0;
    let mut next = vec![Bin::default(); n * 2];
    for (i, bin) in table.iter().enumerate() {
        let (lo, hi) = if bin.tree {
            let (lo, hi): (Vec<_>, Vec<_>) = bin.nodes.iter().partition(|&&(h, _)| !high(h));
            let tree = |nodes: Vec<_>| Bin { tree: nodes.len() > UNTREEIFY_THRESHOLD, nodes };
            (tree(lo), tree(hi))
        } else {
            let (lo, hi) = split_list(&bin.nodes, high);
            (Bin { tree: false, nodes: lo }, Bin { tree: false, nodes: hi })
        };
        next[i] = lo;
        next[i + n] = hi;
    }
    next
}

/// The nodes before the final same-half run are prepended (reversed) onto their half.
fn split_list<'a>(nodes: &[Node<'a>], high: impl Fn(u32) -> bool) -> (Vec<Node<'a>>, Vec<Node<'a>>) {
    let (mut lo, mut hi) = (Vec::new(), Vec::new());
    let Some(&(first, _)) = nodes.first() else { return (lo, hi) };
    let (mut last_run, mut run_bit) = (0, high(first));
    for (j, &(h, _)) in nodes.iter().enumerate().skip(1) {
        if high(h) != run_bit {
            run_bit = high(h);
            last_run = j;
        }
    }
    if run_bit {
        hi.extend_from_slice(&nodes[last_run..])
    } else {
        lo.extend_from_slice(&nodes[last_run..])
    }
    for &node in &nodes[..last_run] {
        if high(node.0) { hi.insert(0, node) } else { lo.insert(0, node) }
    }
    (lo, hi)
}
