#!/bin/bash
# usage: testbox_cpu_all.sh <fixture dir> [hz=199] [binary] — threads, plain, perf and sched passes of testbox_cpu.sh.
D=$(dirname "$0")
bash "$D/testbox_cpu.sh" threads "$1" "$3" | head -20
bash "$D/testbox_cpu.sh" plain "$1" "$3"
bash "$D/testbox_cpu.sh" perf "$1" "$3" "${2:-199}"
bash "$D/testbox_cpu.sh" sched "$1" "$3" 2>&1 | tail -3
uptime
