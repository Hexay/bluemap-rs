#!/bin/bash
# usage: cpu_queries.sh <stacks.folded> — the drill-downs of docs/20 (areas, then what runs under each render area).
D=$(dirname "$0"); F=$1
python3 "$D/cpu_areas.py" "$F" 10
for q in "sort_by_material 0 8" "TileModel>::write_prbm 0 10" "bm_render::resource::render$ 0 25" \
         "block_pass::fully_culled$ 0 8" "block_pass::render$ 1 15" "Volume>::fill 0 12" "World>::load_area 2 12" \
         "bm_engine::resources|StateCache>::new 2 12"; do
  set -- $q
  echo "== $1"; python3 "$D/under.py" "$F" "$1" "$2" "$3"
done
