#!/usr/bin/env python3
"""TEMPORARY: emit the docs-layer CTOR_ALIASES table — aliases mapping
the layer-229 canonical ids to the published-223 ids, keyed by the
union type that owns each predicate. Reads tl.json (official 223 JSON)
and tools/schema.tl (229) to compute the divergence."""

import json
import re
import sys

json_text = open("tl.json", encoding="utf-8").read()
doc = json.loads(json_text)
pairs = {}
for section in ("constructors", "methods"):
    for entry in doc.get(section, []):
        name = entry.get("predicate") or entry.get("method")
        pid = int(entry["id"])
        if pid < 0:
            pid += 4294967296
        pairs[name] = pid

tl_text = open("tools/schema.tl", encoding="utf-8").read()
fresh = {}
for m in re.finditer(r"^([a-zA-Z0-9_.]+)#([0-9a-f]{8})", tl_text, re.M):
    fresh[m.group(1)] = int(m.group(2), 16)

# predicate -> owning union type (from the fresh 229 schema)
owner = {}
for m in re.finditer(r"^([a-zA-Z0-9_.]+)#([0-9a-f]{8})[^=]*= ([A-Za-z0-9_.]+);", tl_text, re.M):
    owner[m.group(1)] = m.group(3)

# Also need the 223 ownership for predicates that only exist in the 223
# JSON: same predicate name usually belongs to the same union. The JSON
# carries "type".
jtype = {}
for section in ("constructors", "methods"):
    for entry in doc.get(section, []):
        jtype[entry.get("predicate") or entry.get("method")] = entry["type"]

aliases = {}
for pred, id229 in fresh.items():
    id223 = pairs.get(pred)
    if id223 is None or id223 == id229:
        continue
    tname = owner.get(pred) or jtype.get(pred)
    if tname is None:
        print(f"// no owner for {pred}", file=sys.stderr)
        continue
    if pred.startswith("messages.") or pred.startswith("channels.") or \
       pred.startswith("stories.") or pred.startswith("contacts.") or \
       pred.startswith("payments.") or pred.startswith("account.") or \
       pred.startswith("channels."):
        continue  # methods: builders, never parsed
    tkey = tname.split(".")[-1]  # messages.Dialogs -> Dialogs
    aliases.setdefault(tkey, {})[pred] = (id229, id223)

out = ["CTOR_ALIASES = {"]
for tname in sorted(aliases):
    out.append(f'    "{tname}": {{')
    for pred in sorted(aliases[tname]):
        id229, id223 = aliases[tname][pred]
        out.append(f'        {id223:#010x}: "{pred}",  // 223 id; 229 = {id229:#010x}')
    out.append("    },")
out.append("}")
print("\n".join(out))
