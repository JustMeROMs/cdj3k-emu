#!/usr/bin/env python3
from __future__ import annotations
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
roots = [ROOT / 'guest', ROOT / 'initramfs-patch']
files = []
for base in roots:
    for p in base.rglob('*'):
        if p.is_file():
            files.append(p)
files.append(ROOT / 'docker' / 'Dockerfile')

h = hashlib.sha256()
for p in sorted(files, key=lambda x: x.relative_to(ROOT).as_posix()):
    rel = p.relative_to(ROOT).as_posix().encode('utf-8')
    data = p.read_bytes()
    h.update(len(rel).to_bytes(4, 'little'))
    h.update(rel)
    h.update(len(data).to_bytes(8, 'little'))
    h.update(data)
print(h.hexdigest())
