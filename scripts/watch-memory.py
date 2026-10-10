#!/usr/bin/env python3
"""Watch a running game's memory at a few addresses and print every change.

  scripts/watch-memory.py <exe name or pid> <mapping json | addr[:size]...>

Given a record-mode mapping file it watches that achievement's remaining
candidates, which makes it quick to see which one really tracks the thing
(e.g. transform 3 times: the counter goes up by exactly 3).
"""
import json, os, re, subprocess, sys, time


def find_pid(what):
    if what.isdigit():
        return int(what)
    for p in os.listdir("/proc"):
        if p.isdigit():
            try:
                if what.lower() in open(f"/proc/{p}/cmdline", "rb").read().decode("utf-8", "ignore").lower() and "watch-memory" not in open(f"/proc/{p}/cmdline").read():
                    return int(p)
            except OSError:
                pass
    sys.exit(f"no running process matching {what}")


def main():
    pid = find_pid(sys.argv[1])
    targets = []
    for a in sys.argv[2:]:
        if a.endswith(".json"):
            targets += [(c["addr"], c["size"]) for c in json.load(open(a))["candidates"]]
        else:
            addr, _, size = a.partition(":")
            targets.append((int(addr, 16), int(size or 1)))
    mem = open(f"/proc/{pid}/mem", "rb", buffering=0)
    def read(addr, size):
        mem.seek(addr)
        return int.from_bytes(mem.read(size), "little")
    last = {t: read(*t) for t in targets}
    counts = {t: 0 for t in targets}
    print(f"watching {len(targets)} addresses in pid {pid}; Ctrl+C to stop")
    try:
        while True:
            time.sleep(0.1)
            for t in targets:
                v = read(*t)
                if v != last[t]:
                    counts[t] += 1
                    print(f"{time.strftime('%H:%M:%S')}  {t[0]:08x}  {last[t]} -> {v}", flush=True)
                    last[t] = v
    except KeyboardInterrupt:
        pass
    print("\nchanges per address:")
    for t, n in sorted(counts.items(), key=lambda x: x[1]):
        print(f"  {t[0]:08x}  {n}")


if __name__ == "__main__":
    main()
