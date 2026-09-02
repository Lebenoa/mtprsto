"""Build the true published .tl schema for a target layer.

core.telegram.org's /schema endpoints ignore ?layer= and serve the last
FULLY published dialect, while the per-layer sections of the changelog
page (core.telegram.org/api/layers) carry a complete .tl dump of every
constructor/method changed at that layer. The published dialect for any
documented layer is therefore: served base schema + the layer diffs in
ascending order.

Usage:
    python tools/update_schema.py 225
        scrape base + layers page, apply diffs for base+1..225, write
        tools/schema_l225.tl (+ .meta.json sidecar)
    python tools/update_schema.py 226 --diff-file new.tl
        apply a hand-provided diff fragment instead of scraping the
        layers page (one layer step on top of the served base)
    python tools/update_schema.py --audit-aliases
        re-check CTOR_ALIASES in gentl.py: every alias must be
        shape-compatible with its 225 ctor (flag-gated additions only),
        or be a wire-verified draft id kept deliberately

Offline flags: --html FILE (cached /api/layers), --base-json FILE
(cached /schema JSON), --base-layer N (skip layer auto-detection).
"""
import argparse
import datetime
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import gentl  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCHEMA_JSON_URL = "https://core.telegram.org/schema/json"
LAYERS_URL = "https://core.telegram.org/api/layers"
USER_AGENT = "mtprsto-gentl"

DECL_RE = re.compile(r"^([a-zA-Z][\w.]*)#([0-9a-fA-F]+)\s")
TRAVERSE_METHOD_RE = re.compile(
    r"traverseMethod(?:Result|Call)\{name:\s*([\w.]+)")


def fetch(url):
    import urllib.request
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    return urllib.request.urlopen(req, timeout=60).read()


def unescape_pre(pre_html):
    t = re.sub(r"<[^>]+>", "", pre_html)
    for a, b in (("&lt;", "<"), ("&gt;", ">"), ("&quot;", '"'),
                 ("&#39;", "'"), ("&amp;", "&")):
        t = t.replace(a, b)
    return t


def section_for_layer(doc, layer):
    i = doc.find(f"Layer {layer}")
    if i < 0:
        return None
    j = doc.find(f"Layer {layer - 1}", i + 10)
    return doc[i:j if j > 0 else i + 200_000]


def diff_for_layer(doc, layer):
    """Extract one layer's diff from the changelog page.

    Returns (lines, methods_in_traverse) where lines is the ordered .tl
    fragment (may contain a ---functions--- separator) and
    methods_in_traverse names methods the section classifies explicitly.
    """
    seg = section_for_layer(doc, layer)
    if seg is None:
        raise SystemExit(f"no changelog section for layer {layer}")
    pres = re.findall(r"<pre>(.*?)</pre>", seg, re.S)
    big = [p for p in pres if len(p) > 3000]
    if len(big) != 1:
        raise SystemExit(f"layer {layer}: expected 1 schema dump "
                         f"<pre>, got {len(big)}")
    lines = []
    for ln in unescape_pre(big[0]).splitlines():
        ln = ln.strip()
        if ln and not ln.startswith("//"):
            lines.append(ln)
    methods = set()
    for p in pres:
        for m in TRAVERSE_METHOD_RE.finditer(unescape_pre(p)):
            methods.add(m.group(1))
    return lines, methods


def apply_diffs(ctor_lines, method_lines, diffs):
    """Apply ordered (name -> full .tl line) diffs onto the schema.

    Replacements land in place; brand-new items are appended to their
    section. Classification for new items: the dump's own
    ---functions--- separator, then the section's traverseMethod*
    lines, then the draft schema, then a loud error.
    """
    draft = gentl.parse_tl(open(os.path.join(ROOT, "tools", "schema.tl"),
                                encoding="utf-8").read())
    ctor_pos = {ln.split("#", 1)[0]: i for i, ln in enumerate(ctor_lines)}
    method_pos = {ln.split("#", 1)[0]: i for i, ln in enumerate(method_lines)}

    replaced = 0
    for layer, lines, in_functions, traverse in diffs:
        for ln in lines:
            if ln.startswith("---"):
                in_functions = not in_functions
                continue
            m = DECL_RE.match(ln)
            if not m:
                raise SystemExit(f"layer {layer}: unparseable diff line: "
                                 f"{ln[:80]}")
            name = m.group(1)
            if name in ctor_pos:
                ctor_lines[ctor_pos[name]] = ln
                replaced += 1
            elif name in method_pos:
                method_lines[method_pos[name]] = ln
                replaced += 1
            else:
                # brand new: classify, cross-checking every signal,
                # then append immediately so a later layer can
                # re-issue it in place
                d = draft.get(name)
                draft_says_fn = d is not None and d.is_function
                is_fn = in_functions or name in traverse
                if d is None and not is_fn:
                    raise SystemExit(
                        f"layer {layer}: cannot classify new item {name}")
                if is_fn or draft_says_fn:
                    print(f"  + method (layer {layer}): {ln.split()[0]}")
                    method_pos[name] = len(method_lines)
                    method_lines.append(ln)
                else:
                    print(f"  + ctor (layer {layer}): {ln.split()[0]}")
                    ctor_pos[name] = len(ctor_lines)
                    ctor_lines.append(ln)
                replaced += 1
    return replaced


def json_to_tl(base):
    def dec(s):
        return int(s) & 0xFFFFFFFF

    def fmt(params):
        return " ".join(f"{p['name']}:{p['type']}" for p in params)

    ctor_lines = [f"{c['predicate']}#{dec(c['id']):08x} {fmt(c['params'])}"
                  f" = {c['type']};" for c in base["constructors"]]
    method_lines = [f"{m['method']}#{dec(m['id']):08x} {fmt(m['params'])}"
                    f" = {m['type']};" for m in base["methods"]]
    return ctor_lines, method_lines


def build(target, out_path, html_path, base_json_path, base_layer,
          diff_file):
    if base_json_path:
        base_bytes = open(base_json_path, "rb").read()
        base_src = base_json_path
    else:
        base_bytes = fetch(SCHEMA_JSON_URL)
        base_src = SCHEMA_JSON_URL
    base = json.loads(base_bytes)

    if base_layer is None:
        base_layer = gentl.fetch_published_layer()
        if base_layer is None:
            raise SystemExit("could not auto-detect the served base "
                             "layer; pass --base-layer")
    print(f"base: {base_src} (published layer {base_layer}, "
          f"{len(base['constructors'])} ctors, "
          f"{len(base['methods'])} methods)")

    diffs = []
    if diff_file:
        lines = [ln.strip()
                 for ln in open(diff_file, encoding="utf-8").readlines()]
        diffs.append((target,
                      [ln for ln in lines if ln and not ln.startswith("//")],
                      False, set()))
        print(f"diff: {diff_file} (as layer {target})")
    else:
        if html_path:
            doc = open(html_path, encoding="utf-8").read()
        else:
            doc = fetch(LAYERS_URL).decode("utf-8")
        for layer in range(base_layer + 1, target + 1):
            lines, traverse = diff_for_layer(doc, layer)
            diffs.append((layer, lines, False, traverse))
            print(f"diff: layer {layer} ({len(lines)} lines, "
                  f"{len(traverse)} traverse methods)")

    ctor_lines, method_lines = json_to_tl(base)
    replaced = apply_diffs(ctor_lines, method_lines, diffs)

    header = (
        f"// TL schema — published layer {target}.\n"
        f"// Assembled: {base_src} (serves the last fully published\n"
        f"// dialect, {base_layer}; the endpoint ignores ?layer=)\n"
        f"// + the layer diffs for {base_layer + 1}..{target} from\n"
        f"// {LAYERS_URL}.\n"
        f"// Regenerate with: python tools/update_schema.py {target}\n"
        "\n"
    )
    footer = f"\n//////////\n// Layer {target}\n"
    text = (header + "\n".join(ctor_lines) + "\n\n---functions---\n\n"
            + "\n".join(method_lines) + footer)
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)

    # sidecar metadata
    meta = {
        "base_layer": base_layer,
        "base_source": base_src,
        "applied_layers": [d[0] for d in diffs],
        "diff_file": diff_file,
        "generated_utc": datetime.datetime.now(
            datetime.timezone.utc).isoformat(timespec="seconds"),
        "bytes": len(text),
        "constructors": len(ctor_lines),
        "methods": len(method_lines),
        "replaced": replaced,
    }
    with open(out_path + ".meta.json", "w", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
        f.write("\n")

    # round-trip: gentl must parse what we built, and every diff line
    # must survive with its exact id
    ctors = gentl.parse_tl(text)
    fns = [c for c in ctors.values() if c.is_function]
    print(f"wrote {out_path}: {len(ctor_lines)} ctors, "
          f"{len(method_lines)} methods (replaced {replaced})")
    print(f"gentl round-trip: {len(ctors)} decls "
          f"({len(ctors) - len(fns)} ctors, {len(fns)} functions)")
    missing = []
    final_lines = {}  # name -> (id, layer): later layers supersede earlier
    for layer, lines, _fn, _tr in diffs:
        for ln in lines:
            if ln.startswith("---"):
                continue
            m = DECL_RE.match(ln)
            final_lines[m.group(1)] = (int(m.group(2), 16), layer)
    for name, (want_id, layer) in final_lines.items():
        got = ctors.get(name)
        if got is None or got.id != want_id:
            missing.append((layer, name))
    if missing:
        for layer, name in missing:
            print(f"MISMATCH after round-trip (layer {layer}): {name}")
        raise SystemExit(1)
    print("round-trip verified: every diff line present with its id")


def audit_aliases():
    """Every alias in CTOR_ALIASES must either be shape-compatible with
    the schema ctor it maps to (flag-gated additions only), or carry a
    comment marking it a wire-verified draft id."""
    schema_path = os.path.join(ROOT, "tools", "schema_l225.tl")
    schema = gentl.parse_tl(open(schema_path, encoding="utf-8").read())
    base = json.load(open(os.path.join(ROOT, "tl.json"), encoding="utf-8"))
    cons223 = {c["predicate"]: c for c in base["constructors"]}
    draft = gentl.parse_tl(open(os.path.join(ROOT, "tools", "schema.tl"),
                                encoding="utf-8").read())

    def fields_from_json(c):
        out = []
        for p in c["params"]:
            cond = None
            t = p["type"]
            if "?" in t:
                cond, t = t.split("?", 1)
            out.append((p["name"], t, cond))
        return out

    def fields_from_ctor(c):
        return [(f.name, f.tl_type,
                 f"{f.flags_word}.{f.flag_bit}" if f.flags_word else None)
                for f in c.fields]

    def compatible(old, new):
        i = 0
        for f in new:
            if i < len(old) and old[i] == f:
                i += 1
            elif f[2] is None:
                return False, f"mandatory {f[0]}:{f[1]} not in old"
        if i != len(old):
            return False, f"old field {old[i]} missing/different in new"
        return True, "ok"

    ok = True
    for tname, entries in gentl.CTOR_ALIASES.items():
        for aid, pred in entries.items():
            c_new = schema.get(pred)
            if c_new is None or c_new.is_function:
                print(f"?? {tname} {aid:#010x}: {pred} not a ctor of the "
                      f"schema")
                continue
            src223, src_draft = cons223.get(pred), draft.get(pred)
            if src223 and (int(src223["id"]) & 0xFFFFFFFF) == aid:
                old, tag = fields_from_json(src223), "223"
            elif (src_draft is not None and not src_draft.is_function
                  and src_draft.id == aid):
                old, tag = fields_from_ctor(src_draft), "draft"
            else:
                print(f"?? {tname} {aid:#010x}: source id not found for "
                      f"{pred}")
                continue
            good, why = compatible(old, fields_from_ctor(c_new))
            if good:
                print(f"ok {tname} {aid:#010x} {pred} ({tag}, "
                      f"shape-compatible)")
            elif tag == "draft":
                print(f"KEEP (wire-verified draft; incompatible: {why}) "
                      f"{tname} {aid:#010x} {pred}")
            else:
                ok = False
                print(f"DROP {tname} {aid:#010x} {pred} — {why}")
    if not ok:
        raise SystemExit(1)


def main():
    ap = argparse.ArgumentParser(
        description="Assemble the published .tl schema for a target "
                    "layer from the served base + changelog diffs")
    ap.add_argument("layer", nargs="?", type=int,
                    help="target published layer (e.g. 225)")
    ap.add_argument("--out", default=None,
                    help="output .tl path "
                         "(default tools/schema_l<LAYER>.tl)")
    ap.add_argument("--html", default=None,
                    help="cached /api/layers HTML instead of fetching")
    ap.add_argument("--base-json", default=None,
                    help="cached /schema JSON instead of fetching")
    ap.add_argument("--base-layer", type=int, default=None,
                    help="override the auto-detected served base layer")
    ap.add_argument("--diff-file", default=None,
                    help="apply this .tl diff fragment instead of "
                         "scraping the layers page")
    ap.add_argument("--audit-aliases", action="store_true",
                    help="check CTOR_ALIASES shape compatibility and "
                         "exit")
    args = ap.parse_args()

    if args.audit_aliases:
        audit_aliases()
        return
    if not args.layer:
        ap.error("need a target layer or --audit-aliases")
    out = args.out or os.path.join(ROOT, "tools",
                                   f"schema_l{args.layer}.tl")
    build(args.layer, out, args.html, args.base_json, args.base_layer,
          args.diff_file)


if __name__ == "__main__":
    main()
