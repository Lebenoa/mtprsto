#!/usr/bin/env python3
"""TEMPORARY: dump 223 shapes needed for the sweep verification."""
import json

doc = json.loads(open('tl.json', encoding='utf-8').read())
wanted = ('inputMediaUploadedDocument', 'documentAttributeFilename',
          'photo', 'photos.photo', 'inputMediaEmpty')
for e in doc.get('constructors', []):
    if e['predicate'] in wanted:
        pid = int(e['id'])
        pid = pid + (1 << 32) if pid < 0 else pid
        params = ', '.join(f"{p['name']}:{p['type']}" for p in e['params'])
        print(f"{e['predicate']} = 0x{pid:08x}")
        print(f"    {params}")
        print()
