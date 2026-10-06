//! Prints a chunk's NBT as an indented tree, arrays summarised.
//! Usage: cargo run -p bm-world --example dump_chunk -- <region dir> <chunk x> <chunk z> [max depth]

use std::path::PathBuf;

use bm_nbt::{Compound, Tag};
use bm_world::region::Region;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, dir, cx, cz, rest @ ..] = args.as_slice() else {
        eprintln!("usage: dump_chunk <region dir> <chunk x> <chunk z> [max depth]");
        std::process::exit(2);
    };
    let (cx, cz): (i32, i32) = (cx.parse().unwrap(), cz.parse().unwrap());
    let depth = rest.first().map_or(6, |d| d.parse().unwrap());
    let region = Region::open(&PathBuf::from(dir), cx.div_euclid(32), cz.div_euclid(32)).unwrap();
    let (mut raw, mut nbt) = (Vec::new(), Vec::new());
    let (lx, lz) = (cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize);
    if !region.read_chunk_into(lx, lz, &mut raw, &mut nbt).unwrap() {
        eprintln!("chunk not generated");
        return;
    }
    println!("timestamp {}, {} bytes", region.timestamp(lx, lz), nbt.len());
    print_compound(bm_nbt::read_root(&nbt).unwrap(), 0, depth);
}

fn print_compound(c: Compound, indent: usize, depth: usize) {
    for (name, tag) in c.entries() {
        print!("{:indent$}{}: ", "", String::from_utf8_lossy(name), indent = indent * 2);
        print_tag(tag, indent, depth);
    }
}

fn print_tag(tag: Tag, indent: usize, depth: usize) {
    match tag {
        Tag::Compound(c) if indent < depth => {
            println!("{{");
            print_compound(c, indent + 1, depth);
        }
        Tag::List(l) if indent < depth && !l.is_empty() => {
            println!("list[{}] of type {}", l.len(), l.elem_type());
            for (i, t) in l.iter().take(3).enumerate() {
                print!("{:indent$}[{i}] ", "", indent = (indent + 1) * 2);
                print_tag(t, indent + 1, depth);
            }
        }
        Tag::String(s) => println!("{:?}", String::from_utf8_lossy(s)),
        Tag::LongArray(a) => println!("long[{}]", a.len()),
        Tag::IntArray(a) => println!("int[{}]", a.len()),
        Tag::ByteArray(a) => println!("byte[{}]", a.len()),
        Tag::Compound(_) => println!("{{…}}"),
        Tag::List(l) => println!("list[{}]", l.len()),
        other => println!("{other:?}"),
    }
}
