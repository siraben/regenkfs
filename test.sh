#!/usr/bin/env sh
set -e

cargo run --features "c-undef" -- out-r.rom ./test
genkfs out-c.rom ./test
xxd out-c.rom > out-c.txt
xxd out-r.rom > out-r.txt
vimdiff out-c.txt out-r.txt
