"""Content-hash dedup of hires tiles (within map and across maps) and empty-tile count."""
import collections
import hashlib
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402

by_hash = collections.defaultdict(list)
empty = 0
for mapid, path in prbm.corpus():
    gz, raw = prbm.load(path)
    empty += int.from_bytes(raw[2:5], "little") == 0
    by_hash[hashlib.sha256(raw).hexdigest()].append((mapid, len(gz)))
groups = [v for v in by_hash.values() if len(v) > 1]
saved = sum(sz for v in groups for _, sz in v[1:])
within = sum(len(v) - len({m for m, _ in v}) for v in groups)
total = sum(sz for v in by_hash.values() for _, sz in v)
empties = [v for v in groups if v[0][1] < 200]
print(f"tiles {sum(len(v) for v in by_hash.values())} unique {len(by_hash)} empty {empty} dup groups {len(groups)} "
      f"redundant tiles {sum(len(v) - 1 for v in groups)} (within-map {within}) saved gz {saved / 1e6:.2f}MB "
      f"of {total / 1e6:.1f}MB = {100 * saved / total:.1f}%; tiny(<200B) dup groups {len(empties)}")
pairs = collections.Counter(tuple(sorted({m for m, _ in v})) for v in groups)
print("map sets sharing tiles:", pairs.most_common(6))
