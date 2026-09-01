#!/usr/bin/env python3
"""Generate Rust TL parsers from a Telegram API .tl schema.

The .tl schema (e.g. tdlib/td's telegram_api.tl, tdesktop's api.tl) is the
official field-order specification: every constructor lists its fields in
exact wire order, with `flags.N?` conditional prefixes.

Usage:
  python tools/gentl.py <schema.tl> --type user userEmpty channel ...
                        [--out src/types/gen.rs] [--diff]

Generates, for each requested type:
  - a struct (single-constructor types) or enum (union types) with variants
  - a `read_from(&mut TLReader)` decoder honoring flags/vectors/nested types
  - ctor-id constants named after the predicate

Also supports `--diff`: report ctor ids present in the schema but different
in our hand-maintained src/types/constructors.rs (stale-constant detector).
Only stdlib is used; rust codegen targets mtprsto's TLReader API.
"""

import argparse
import re
import sys

# TL built-in types -> (rust type, read expression on TLReader)
BUILTINS = {
    "int": ("i32", "read_i32"),
    "long": ("i64", "read_i64"),
    "int128": ("[u8; 16]", None),  # read raw 16 bytes
    "int256": ("[u8; 32]", None),
    "double": ("f64", None),  # read_u64 + from_bits
    "string": ("String", None),  # read_bytes + from_utf8
    "bytes": ("Vec<u8>", "read_bytes"),
    "#": ("i32", "read_i32"),
    "true": ("bool", None),  # flag bits carry no bytes
    "Bool": ("bool", None),  # ctor-serialized boolTrue/boolFalse
}

# Result types that are TL built-ins — never generate Rust types for them.
BUILTIN_RESULT_TYPES = {
    "Bool", "True", "Vector", "X", "Int", "Long", "Double", "String",
    "Bytes", "Int128", "Int256", "Jsonobject", "Jsonvalue",
}

CTOR_NAME_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9_.]*$")
FIELD_NAME_RE = re.compile(r"^[a-z][a-z0-9_]*$")

RUST_KEYWORDS = {
    "type", "ref", "match", "static", "const", "fn", "impl", "let", "mut",
    "move", "pub", "struct", "enum", "trait", "use", "where", "self", "as",
    "box", "dyn", "else", "false", "for", "if", "in", "loop", "return",
    "true", "unsafe", "while", "await", "async", "crate", "final", "override",
    "priv", "typeof", "unsized", "virtual", "yield", "try", "do", "abstract",
    "become", "macro",
}


def field_ident(name):
    # `self`/`Self`/`crate`/`super` cannot even be raw identifiers.
    if name in ("self", "Self", "crate", "super"):
        return name + "_"
    return "r#" + name if name in RUST_KEYWORDS else name


def snake(name):
    name = name.replace(".", "_")
    out = []
    for i, ch in enumerate(name):
        if ch.isupper():
            if i > 0 and (not name[i - 1].isupper() or (i + 1 < len(name) and name[i + 1].islower())):
                out.append("_")
            out.append(ch.lower())
        else:
            out.append(ch)
    return "".join(out)


def pascal(name):
    # 'user' -> 'User', 'inputPeerUser' -> 'InputPeerUser',
    # 'users_getUsers' -> 'UsersGetUsers' (underscores fold away)
    parts = re.split(r"[._]", name)
    return "".join(p[:1].upper() + p[1:] for p in parts if p)


def rust_type_name(tname):
    # 'users.UserFull' -> 'UsersUserFull' so namespace containers never
    # collide with the bare object type ('UserFull').
    return pascal(tname.replace(".", "_"))


class Field:
    def __init__(self, name, tl_type, flags_word, flag_bit, is_vector):
        self.name = name
        self.tl_type = tl_type
        self.flags_word = flags_word  # 'flags' | 'flags2' | None
        self.flag_bit = flag_bit
        self.is_vector = is_vector


class Ctor:
    def __init__(self, predicate, cid, type_name, fields):
        self.predicate = predicate
        self.id = cid
        self.type_name = type_name
        self.fields = fields


def parse_tl(text):
    ctors = {}
    in_functions = False
    # Strip comments
    text = re.sub(r"//[^\n]*", "", text)
    # Each declaration ends with ' = TypeName;' possibly across lines.
    for line in text.splitlines():
        if line.startswith("---functions---"):
            in_functions = True
        if line.startswith("---types---"):
            in_functions = False
        for m in re.finditer(r"([a-zA-Z][\w.]*)#([0-9a-fA-F]+)\s+([^=]*?)=\s*([\w.]+)\s*;", line):
            pred, cid, body, tname = m.group(1), int(m.group(2), 16), m.group(3), m.group(4)
            if not CTOR_NAME_RE.match(pred):
                continue
            fields = []
            tokens = body.split()
            for tok in tokens:
                if ":" not in tok:
                    continue
                fname, ftype = tok.split(":", 1)
                if not FIELD_NAME_RE.match(fname):
                    continue
                flags_word, flag_bit, is_vec = None, None, False
                if "?" in ftype:
                    cond, ftype = ftype.split("?", 1)
                    fw, bit = cond.split(".")
                    flags_word, flag_bit = fw, int(bit)
                if ftype.startswith("Vector<") and ftype.endswith(">"):
                    is_vec = True
                    ftype = ftype[len("Vector<"):-1]
                elif ftype.startswith("vector<"):
                    is_vec = True
                    ftype = ftype[len("vector<"):-1]
                fields.append(Field(fname, ftype, flags_word, flag_bit, is_vec))
            c = Ctor(pred, cid, tname, fields)
            c.is_function = in_functions
            ctors[pred] = c
    return ctors


def rust_read_expr(field, ctors, indent):
    t = field.tl_type
    if field.is_vector:
        inner = field.tl_type
        if inner in BUILTINS:
            rust_t, reader = BUILTINS[inner]
            if reader is None:
                return None
            return (
                f"{{ let n = r.{reader}... }}"
            )
        return None
    return None


types_by_name_global = {}
generatable_global = {}
writable_global = {}
outer_type_global = None


def compute_writable(ctors, types_by_name, generatable):
    """Types usable on the WRITE side: single-ctor structs whose every
    field is writable (recursively). Unions have no write_to."""
    WRITABLE_BUILTINS = {"int", "long", "double", "string", "bytes",
                         "int128", "int256", "#", "Bool"}
    memo = {}
    def ok(tname, stack):
        if tname in WRITABLE_BUILTINS:
            return True
        if tname in memo:
            return memo[tname]
        if tname in stack:
            return False
        stack = stack | {tname}
        group = types_by_name.get(tname, [])
        if len(group) != 1:
            memo[tname] = False
            return False
        c = group[0]
        for f in c.fields:
            ft = f.tl_type
            if f.is_vector:
                if ft in WRITABLE_BUILTINS or (ft in types_by_name and ok(ft, stack)):
                    continue
                memo[tname] = False
                return False
            if ft in WRITABLE_BUILTINS:
                continue
            if ft in types_by_name and ok(ft, stack):
                continue
            memo[tname] = False
            return False
        memo[tname] = True
        return True
    for t in types_by_name:
        ok(t, frozenset())
    return memo


def rust_param_type(f, types_by_name, generatable):
    """Rust parameter type for a function/field, or None if unsupported."""
    t = f.tl_type
    if f.is_vector:
        if t in ("int", "#"):
            return "&[i32]"
        if t == "long":
            return "&[i64]"
        if t == "string":
            return "&[Vec<u8>]"
        if t == "bytes":
            return "&[Vec<u8>]"
        if t in types_by_name and t in generatable and generatable[t]:
            return f"&[{rust_type_name(t)}]"
        return None
    if t in ("int", "#"):
        return "i32"
    if t == "long":
        return "i64"
    if t == "double":
        return "f64"
    if t == "int128":
        return "[u8; 16]"
    if t == "int256":
        return "[u8; 32]"
    if t == "string":
        return "&str"
    if t == "bytes":
        return "&[u8]"
    if t == "true":
        return "bool"
    if t == "Bool":
        return "bool"
    if t in writable_global and writable_global[t]:
        return f"&{rust_type_name(t)}"
    return None


def gen_field_write(f, ind, vp="", borrow=False):
    """Statements writing field f to `w`. vp = value prefix ('' for fn
    params, 'self.' inside write_to)."""
    pad = " " * ind
    t = f.tl_type
    n = field_ident(f.name)
    if f.is_vector:
        if t in ("int", "#"):
            return [
                f"{pad}w.write_u32(crate::serialize::VECTOR);",
                f"{pad}w.write_i32({vp}{n}.len() as i32);",
                f"{pad}for item in {vp}{n}.iter().copied() {{",
                f"{pad}    w.write_i32(item);",
                f"{pad}}}",
            ]
        if t == "long":
            return [
                f"{pad}w.write_u32(crate::serialize::VECTOR);",
                f"{pad}w.write_i32({vp}{n}.len() as i32);",
                f"{pad}for item in {vp}{n}.iter().copied() {{",
                f"{pad}    w.write_i64(item);",
                f"{pad}}}",
            ]
        if t in ("string", "bytes"):
            return [
                f"{pad}w.write_u32(crate::serialize::VECTOR);",
                f"{pad}w.write_i32({vp}{n}.len() as i32);",
                f"{pad}for item in {vp}{n}.iter() {{",
                f"{pad}    w.write_bytes(item);",
                f"{pad}}}",
            ]
        if t in writable_global and writable_global[t]:
            iter_expr = f"{vp}{n}.iter()" if vp == "self." else f"{vp}{n}.iter()"
            return [
                f"{pad}w.write_u32(crate::serialize::VECTOR);",
                f"{pad}w.write_i32({vp}{n}.len() as i32);",
                f"{pad}for item in {vp}{n}.iter() {{",
                f"{pad}    item.write_to(w);",
                f"{pad}}}",
            ]
        return None
    if t in ("int", "#"):
        return [f"{pad}w.write_i32({vp}{n});"]
    if t == "long":
        return [f"{pad}w.write_i64({vp}{n});"]
    if t == "double":
        return [f"{pad}w.write_double({vp}{n});"]
    if t == "string":
        return [f"{pad}w.write_bytes({vp}{n}.as_bytes());"]
    if t == "bytes":
        # owned values (struct fields, cloned conditionals) need a borrow;
        # fn params are already &[u8]
        arg = f"&{vp}{n}" if borrow else f"{vp}{n}"
        return [f"{pad}w.write_bytes({arg});"]
    if t == "int128":
        return [f"{pad}w.write_raw_bytes(&{vp}{n});"]
    if t == "int256":
        return [f"{pad}w.write_raw_bytes(&{vp}{n});"]
    if t == "Bool":
        return [f"{pad}if {vp}{n} {{ w.write_bool_true(); }} else {{ w.write_bool_false(); }}"]
    if t in writable_global and writable_global[t]:
        return [f"{pad}{vp}{n}.write_to(w);"]
    return None


def compat_fix_write(stmts, rust_types):
    """Unwrap newtype fields in generated write_to statements."""
    if not compat_global or stmts is None:
        return stmts
    wrapped = {f for f, t in rust_types.items()
               if t in NEWTYPE_NAMES
               or (t.startswith("Option<") and t[7:-1] in NEWTYPE_NAMES)}
    if not wrapped:
        return stmts
    out = []
    for st in stmts:
        bare = st.rstrip("\r\n")
        eol = st[len(bare):]
        for f in wrapped:
            ident = field_ident(f)
            # self.field style
            for pat, rep in (
                (f"self.{ident});", f"self.{ident}.0);"),
                (f"self.{ident}.as_bytes());", f"self.{ident}.0.as_bytes());"),
            ):
                bare = bare.replace(pat, rep)
            # if-let Some(ident) = ... clone branch: bare local style
            for wtr in ("write_i64", "write_i32"):
                bare = bare.replace(
                    f"w.{wtr}({ident});",
                    f"w.{wtr}({ident}.0);",
                )
        out.append(bare + eol)
    return out


NEWTYPE_NAMES = {"UserId", "ChatId", "ChannelId", "AccessHash", "MsgId",
                 "PhotoId", "DocumentId"}


def gen_write_to(c, rust_types):
    """Body of write_to for a single-ctor type. None if any required
    (non-conditional) field is unwritable."""
    stmts = [f'    w.write_u32({snake(c.predicate.replace(".", "_")).upper()}_ID);']
    flag_bits = {}
    for f in c.fields:
        if f.flags_word and f.tl_type != "#":
            flag_bits.setdefault(f.flags_word, []).append(f)
    for fw in flag_bits:
        expr = " | ".join(
            (
                f"if self.{field_ident(f.name)} {{ 1 << {f.flag_bit} }} else {{ 0 }}"
                if f.tl_type == "true"
                else f"if self.{field_ident(f.name)}.is_some() {{ 1 << {f.flag_bit} }} else {{ 0 }}"
            )
            for f in flag_bits[fw]
        )
        stmts.append(f"    let {fw}: i32 = " + (expr or "0") + ";")
        stmts.append(f"    w.write_i32({fw});")
    for f in c.fields:
        if f.tl_type == "#":
            continue
        n = field_ident(f.name)
        if f.flags_word:
            if f.tl_type == "true":
                continue  # bit only
            rt = rust_types.get(f.name, "")
            inner = rt[7:-1] if rt.startswith("Option<") else rt
            copy_bind = inner in ("i32", "i64", "f64", "bool")
            bind = f"self.{n}" if copy_bind else f"self.{n}.clone()"
            stmts.append(f"    if let Some({n}) = {bind} {{")
            body = gen_field_write(f, 8, vp="", borrow=True)
            if body is None:
                return None
            stmts.extend(body)
            stmts.append("    }")
        else:
            body = gen_field_write(f, 4, vp="self.", borrow=True)
            if body is None:
                return None
            stmts.extend(body)
    return stmts


def gen_response_map(fn_ctors, types_by_name):
    """Emit `expected_response_ctor(method_ctor)` — the schema-derived
    method->response routing the runtime dispatch previously guessed by
    hand. Returns the generated Rust lines (may be empty)."""
    if not fn_ctors:
        return []
    # result type -> ctor ids of genuine type constructors only
    # (parse_tl also files functions under their result type).
    type_ctors = {}
    for tname, group in types_by_name.items():
        type_ctors[tname] = [c.id for c in group if not getattr(c, 'is_function', False)]

    lines = [
        "/// Schema-derived routing: which response constructor(s) each",
        "/// method's result type can arrive as. Replaces the hand-glued",
        "/// response expectations per wrapper.",
        "pub fn expected_response_ctors(method_ctor: u32) -> &'static [u32] {",
        "    match method_ctor {",
    ]
    for c in fn_ctors:
        ids = type_ctors.get(c.type_name, [])
        if not ids:
            continue
        arms = ", ".join(f"0x{i:08x}" for i in ids)
        lines.append(f"        0x{c.id:08x} => &[{arms}],")
    lines.append("        _ => &[],")
    lines.append("    }")
    lines.append("}")
    lines.append("")
    return lines


def gen_function(c):
    """Emit a `build_<name>` request-serialization function. Returns list
    of lines, or None if any field type is unsupported."""
    params = []
    stmts = [f'    let mut w = crate::serialize::TLWriter::new();',
             f'    w.write_u32({snake(c.predicate.replace(".", "_")).upper()}_ID);']
    # flag words first
    flag_bits = {}
    for f in c.fields:
        if f.flags_word and f.tl_type != "#":
            flag_bits.setdefault(f.flags_word, []).append(f)
    arg_names = []
    for f in c.fields:
        if f.tl_type == "#":
            continue
        rt = rust_param_type(f, types_by_name_global, generatable_global)
        if rt is None:
            return None
        n = field_ident(f.name)
        if f.flags_word:
            param = rt if f.tl_type == "true" else f"{rt}"
            if f.tl_type != "true":
                param = f"Option<{rt}>"
            arg_names.append((n, param, f))
        else:
            arg_names.append((n, rt, f))
    # signature (skip flag words — computed)
    sig = []
    for n, rt, f in arg_names:
        if f.tl_type == "true":
            sig.append(f"{n}: bool")
        elif f.flags_word:
            sig.append(f"{n}: {rt}")
        else:
            sig.append(f"{n}: {rt}")
    # flag computations
    for fw, fs in flag_bits.items():
        expr = " | ".join(
            (
                f"if {field_ident(f.name)} {{ 1 << {f.flag_bit} }} else {{ 0 }}"
                if f.tl_type == "true"
                else f"if {field_ident(f.name)}.is_some() {{ 1 << {f.flag_bit} }} else {{ 0 }}"
            )
            for f in fs
        )
        stmts.append(f"    let {fw}: i32 = " + (expr or "0") + ";")
        stmts.append(f"    w.write_i32({fw});")
    # writes
    for f in c.fields:
        if f.tl_type == "#":
            continue
        if f.flags_word:
            if f.tl_type == "true":
                continue
            stmts.append(f"    if let Some({field_ident(f.name)}) = {field_ident(f.name)} {{")
            body = gen_field_write(f, 8)
            if body is None:
                return None
            stmts.extend(body)
            stmts.append("    }")
        else:
            body = gen_field_write(f, 4)
            if body is None:
                return None
            # fn builders own `w` — object writes need &mut
            body = [b.replace(".write_to(w);", ".write_to(&mut w);") for b in body]
            stmts.extend(body)
    stmts.append("    w.into_bytes()")
    header = [
        f"/// `{c.predicate}#{c.id:08x} = {c.type_name}` — request builder.",
        f"/// Returns the serialized method payload (wrap in invoke* at call site).",
        "#[allow(clippy::too_many_arguments)]",
    ]
    lines = header
    lines.append(f"pub fn build_{snake(c.predicate.replace('.', '_'))}(")
    for i, sg in enumerate(sig):
        comma = "," if i < len(sig) - 1 else ""
        lines.append(f"    {sg}{comma}")
    lines.append(") -> Vec<u8> {")
    lines.extend(stmts)
    lines.append("}")
    lines.append("")
    return lines


def compute_needs_box(ctors, types_by_name, generatable):
    """Set of (outer, inner) generated-type pairs where `inner` must be
    Box<>ed inside `outer`'s Rust type.

    A field of type U inside type T needs indirection iff U can reach T
    again through generated-type edges (i.e. U lies on a cycle containing
    T). Blanket-boxing every object field pays a heap alloc per field per
    parse; only cycle members actually need the pointer.
    """
    edges = {}
    for tname, group in types_by_name.items():
        outs = set()
        for c in group:
            for f in c.fields:
                if f.tl_type in BUILTINS or f.tl_type == "#":
                    continue
                if f.tl_type in types_by_name:
                    outs.add(f.tl_type)
        edges[tname] = outs

    # Transitive closure per type (DFS from each; ~350 types, runs once).
    reach = {t: set() for t in edges}
    for t in edges:
        work = list(edges[t])
        seen = set()
        while work:
            u = work.pop()
            if u in seen:
                continue
            seen.add(u)
            work.extend(edges.get(u, ()))
        reach[t] = seen

    needs_box = set()
    for t in types_by_name:
        for u in edges[t]:
            if u in BUILTINS or u == "#":
                continue
            # Box U inside T iff U can reach T again: the field would
            # otherwise make T infinitely sized. Self-recursion (U == T)
            # always needs the pointer; generatability is irrelevant here
            # because recursive types are exactly the ones the
            # can_generate memo marks False.
            if u == t or t in reach[u]:
                needs_box.add((t, u))
    return needs_box


needs_box_global = set()
compat_global = False


def gen_field_read(f, ctors, ind, types_by_name, generatable):
    """Return Rust statements reading field f from `r`. None = unsupported."""
    pad = " " * ind
    t = f.tl_type
    n = field_ident(f.name)
    if f.is_vector:
        if t in ("int", "#", "long"):
            reader = "read_i32" if t in ("int", "#") else "read_i64"
            return [
                f"{pad}let n = r.read_vector_header()?;",
                f"{pad}let mut {n} = Vec::with_capacity(n.max(0) as usize);",
                f"{pad}for _ in 0..n {{",
                f"{pad}    {n}.push(r.{reader}()?);",
                f"{pad}}}",
            ], f"Vec<{BUILTINS[t][0] if t in BUILTINS else 'i64'}>"
        if t in types_by_name and t in generatable and generatable[t]:
            rust_t = rust_type_name(t)
            return [
                f"{pad}let n = r.read_vector_header()?;",
                f"{pad}let mut {n} = Vec::with_capacity(n.max(0) as usize);",
                f"{pad}for _ in 0..n {{",
                f"{pad}    {n}.push({rust_t}::read_from(r)?);",
                f"{pad}}}",
            ], f"Vec<{rust_t}>"
        if t in ("string", "bytes"):
            return [
                f"{pad}let n = r.read_vector_header()?;",
                f"{pad}let mut {n} = Vec::with_capacity(n.max(0) as usize);",
                f"{pad}for _ in 0..n {{",
                f"{pad}    {n}.push(r.read_bytes()?);",
                f"{pad}}}",
            ], "Vec<Vec<u8>>"
        return None, None
    if t in BUILTINS:
        rust_t, reader = BUILTINS[t]
        if t == "true":
            return [f"{pad}let _ = {n}; /* flag bit, no bytes */"], "bool"
        if t == "Bool":
            return [
                f"{pad}let {n} = r.read_u32()? == 0x997275b5; // boolTrue",
            ], "bool"
        if t == "double":
            return [
                f"{pad}let {n} = f64::from_bits(r.read_u64()?);",
            ], "f64"
        if t == "int128":
            return [f"{pad}let {n} = {{ r.skip(16)?; [0u8; 16] }};"], "[u8; 16]"
        if t == "int256":
            return [f"{pad}let {n} = {{ r.skip(32)?; [0u8; 32] }};"], "[u8; 32]"
        if rust_t == "String":
            return [f'{pad}let {n} = String::from_utf8(r.read_bytes()?)?;'], "String"
        if reader is None:
            return None, None
        return [f"{pad}let {n} = r.{reader}()?;"], rust_t
    if t in types_by_name and t in generatable and generatable[t]:
        # Box only when the nested type sits on a cycle (see
        # compute_needs_box); everything else inlines, saving a heap
        # allocation per field per parse.
        rust_t = rust_type_name(t)
        if (outer_type_global, t) in needs_box_global:
            return [f"{pad}let {n} = Box::new({rust_t}::read_from(r)?);"], f"Box<{rust_t}>"
        return [f"{pad}let {n} = {rust_t}::read_from(r)?;"], rust_t
    return None, None


def gen_ctor(ctor, ctors, types_by_name, generatable):
    """Generate the body of a read_from for one constructor. Returns (stmts, field_rust_types) or None."""
    stmts = []
    rust_types = {}
    flag_words = {}
    for f in ctor.fields:
        if f.tl_type == "#":
            fw = f.name  # 'flags' or 'flags2'
            flag_words[fw] = True
            stmts.append(f"    let {fw} = r.read_i32()?;")
            rust_types[f.name] = "i32"
            continue

    for f in ctor.fields:
        if f.tl_type == "#":
            continue
        if f.flags_word is not None and f.tl_type != "true":
            # only read when flag set
            res = gen_field_read(f, ctors, 8, types_by_name, generatable)
            if res is None or res[0] is None:
                return None
            body, rt = res
            rust_types[f.name] = rt
            opt = "Option<" + rt + ">" if rt else None
            stmts.append(f"    let {field_ident(f.name)} = if {f.flags_word} & (1 << {f.flag_bit}) != 0 {{")
            stmts.extend(body)
            stmts.append(f"        Some({field_ident(f.name)})")
            stmts.append("    } else {")
            stmts.append("        None")
            stmts.append("    };")
            rust_types[f.name] = opt
        elif f.tl_type == "true":
            # flag bit only — bind the boolean from the flags word
            rust_types[f.name] = "bool"
            stmts.append(
                f"    let {field_ident(f.name)} = "
                f"{f.flags_word} & (1 << {f.flag_bit}) != 0;"
            )
        else:
            res = gen_field_read(f, ctors, 4, types_by_name, generatable)
            if res is None or res[0] is None:
                return None
            body, rt = res
            rust_types[f.name] = rt
            stmts.extend(body)

    if compat_global:
        # Compat pass: wrap id fields in newtypes. Rewrite the `let` to
        # construct the newtype and retype rust_types accordingly. The
        # wrapper ctor id (variant type) comes from the caller profile.
        for f in ctor.fields:
            raw_t = rust_types.get(f.name)
            needs_i32_cast = False
            if raw_t is None:
                continue
            # Unwrap Option<> for the check; Option<i64> ids stay Option.
            is_opt = raw_t.startswith("Option<")
            base = raw_t[7:-1] if is_opt else raw_t
            nt = compat_field_type(ctor.type_name, f.name, base)
            needs_i32_cast = (
                nt is None
                and f.name == "id"
                and base == "i32"
                and ctor.predicate in MSGID_ID_CTOR
            )
            if nt is None and needs_i32_cast:
                nt = "MsgId"
            if nt is None:
                continue
            ident = field_ident(f.name)
            n = f"{nt}({ident})"
            if is_opt:
                stmts_rewrite = []
                for st in stmts:
                    st = st.replace(f"Some({ident})", f"Some({nt}({ident}))")
                    stmts_rewrite.append(st)
                stmts = stmts_rewrite
                rust_types[f.name] = f"Option<{nt}>"
            else:
                # Replace the bare let with a newtype-constructed let.
                # i32 message ids widen to the i64 MsgId payload.
                cast = " as i64" if needs_i32_cast else ""
                stmts.append(f"    let {ident} = {nt}({ident}{cast});")
                rust_types[f.name] = nt
    return stmts, rust_types


def gen_output(ctors, requested, include_functions=False, domain=None):
    global types_by_name_global, generatable_global, writable_global
    global needs_box_global, outer_type_global
    types_by_name = {}
    for c in ctors.values():
        types_by_name.setdefault(c.type_name, []).append(c)

    # A type is generatable only if every non-builtin field type is
    # (recursively). Unions need at least one generatable ctor.
    generatable = {}
    def can_generate(tname, stack):
        if tname in BUILTINS or tname == "#":
            return True
        if tname in generatable:
            return generatable[tname]
        if tname in stack:
            return False
        stack = stack | {tname}
        group = types_by_name.get(tname, [])
        for c in group:
            ok = True
            for f in c.fields:
                ft = f.tl_type
                if f.is_vector:
                    ft = f.tl_type
                if ft in BUILTINS or ft == "#":
                    continue
                if not can_generate(ft, stack):
                    ok = False
                    break
            if ok:
                generatable[tname] = True
                return True
        generatable[tname] = False
        return False
    for t in list(types_by_name):
        can_generate(t, frozenset())
    types_by_name_global = types_by_name
    generatable_global = generatable
    writable_global = compute_writable(ctors, types_by_name, generatable)
    needs_box_global = compute_needs_box(ctors, types_by_name, generatable)

    # Transitive closure: every nested non-builtin type must be generated.
    needed = []
    seen = set(requested)
    queue = list(requested)
    while queue:
        pred = queue.pop()
        c = ctors.get(pred)
        if c is None:
            continue
        needed.append(pred)
        for f in c.fields:
            t = f.tl_type
            if t in BUILTINS or f.tl_type == "#":
                continue
            for dep in types_by_name.get(t, []):
                if dep.predicate not in seen:
                    seen.add(dep.predicate)
                    queue.append(dep.predicate)
    requested = needed
    foreign_types = {}
    if domain:
        def _owner_of(c):
            return (CANONICAL_OWNERS.get(c.type_name)
                    or type_owner_global.get(c.type_name, domain))
        foreign_types = {
            ctors[p].type_name: _owner_of(ctors[p])
            for p in needed if _owner_of(ctors[p]) != domain
        }
        requested = [p for p in requested if _owner_of(ctors[p]) == domain]

    lines = []
    lines.append("//! GENERATED by tools/gentl.py from a Telegram API .tl schema.")
    lines.append("//! Field order and flags come straight from the schema — do not hand-edit.")
    lines.append("//! Unread/unused fields keep their reads so the stream stays aligned.")
    lines.append("#![allow(dead_code, unused_variables, unused_mut)]")
    lines.append("#![allow(unused_imports)]")
    lines.append("#![allow(clippy::enum_variant_names)]")
    lines.append("#![allow(clippy::clone_on_copy)]")
    lines.append("#![allow(clippy::needless_option_as_deref)]")
    lines.append("#![allow(clippy::large_enum_variant)]")
    # Justification lives in the generated file itself: the strict gate
    # is relaxed wholesale for generated wire code only.
    lines.append("// Schema-shaped wire code is generated, never hand-maintained: the")
    lines.append("// strict gate's pedantic/nursery groups and the byte-wrangling")
    lines.append("// classes (casts, wire-int narrowing) are silenced wholesale here")
    lines.append("// instead of in every handwritten module.")
    lines.append("#![allow(clippy::pedantic, clippy::nursery)]")
    lines.append("#![allow(clippy::as_conversions, clippy::cast_sign_loss)]")
    lines.append("#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]")
    lines.append("")
    lines.append("use crate::error::{Error, Result};")
    lines.append("use crate::serialize::TLReader;")
    if compat_global:
        lines.append("use crate::types::{UserId, ChatId, ChannelId, AccessHash, MsgId, PhotoId, DocumentId};")
    if domain and foreign_types:
        by_owner = {}
        for tname, owner in foreign_types.items():
            by_owner.setdefault(owner, []).append(rust_type_name(tname))
        for owner, names in sorted(by_owner.items()):
            lines.append(f"use super::{owner}_gen::{{{', '.join(sorted(names))}}};")
    lines.append("")
    # constants
    for pred in requested:
        c = ctors.get(pred)
        if c is None:
            continue
        lines.append(f"pub const {snake(pred).upper()}_ID: u32 = 0x{c.id:08x};")
    lines.append("")
    # group by result type
    by_type = {}
    fn_ctors = []
    for pred in requested:
        c = ctors.get(pred)
        if c is None:
            print(f"# warning: {pred} not in schema", file=sys.stderr)
            continue
        if getattr(c, "is_function", False):
            fn_ctors.append(c)
            continue
        by_type.setdefault(c.type_name, []).append(c)
    for tname, group in by_type.items():
        if tname in ("Error", "Result"):
            lines.append(f"// TODO type {tname}: name collides with crate::error imports")
            lines.append("")
            continue
        rust_t = rust_type_name(tname)
        if len(group) == 1:
            c = group[0]
            outer_type_global = tname
            body = gen_ctor(c, ctors, types_by_name, generatable)
            if body is None:
                lines.append(f"// TODO {c.predicate}: contains a type this generator does not know")
                lines.append("")
                continue
            stmts, rust_types = body
            lines.append(f"/// `{c.predicate}#{c.id:08x} = {tname}`")
            lines.append("#[derive(Debug, Clone, PartialEq)]")
            lines.append(f"pub struct {rust_t} {{")
            for f in c.fields:
                rt = rust_types.get(f.name)
                if rt is None:
                    continue
                lines.append(f"    pub {field_ident(f.name)}: {rt},")
            lines.append("}")
            lines.append("")
            lines.append(f"impl {rust_t} {{")
            lines.append("    pub fn read_from(r: &mut TLReader) -> Result<Self> {")
            lines.append(f"        let ctor = r.read_u32()?;")
            lines.append(f"        if ctor != {snake(c.predicate.replace(".", "_")).upper()}_ID {{")
            lines.append('            return Err(Error::Serialization(format!(')
            lines.append(f'                "expected {c.predicate}, got {{ctor:#x}}"')
            lines.append("            )));")
            lines.append("        }")
            lines.extend(stmts)
            lines.append(f"        Ok({rust_t} {{")
            for f in c.fields:
                if f.name in rust_types:
                    lines.append(f"            {field_ident(f.name)},")
            lines.append("        })")
            lines.append("    }")
            lines.append("")
            write_body = compat_fix_write(gen_write_to(c, rust_types), rust_types)
            if write_body is not None:
                lines.append("    /// Serialize in schema field order (flags auto-computed).")
                lines.append("    pub fn write_to(&self, w: &mut crate::serialize::TLWriter) {")
                lines.extend(write_body)
                lines.append("    }")
            lines.append("}")
            lines.append("")
        else:
            outer_type_global = tname
            supported = [c for c in group if gen_ctor(c, ctors, types_by_name, generatable) is not None]
            if not supported:
                lines.append(f"// TODO union {tname}: no constructor is generatable "
                             f"({', '.join(c.predicate for c in group)})")
                lines.append("")
                continue
            lines.append(f"/// Union `{tname}` ({len(group)} constructors).")
            lines.append("#[derive(Debug, Clone, PartialEq)]")
            lines.append(f"pub enum {rust_t} {{")
            variants = []
            for c in group:
                outer_type_global = tname
                body = gen_ctor(c, ctors, types_by_name, generatable)
                if body is None:
                    continue
                stmts, rust_types = body
                vn = pascal(c.predicate.replace(".", "_"))
                variants.append((c, vn, stmts, rust_types))
                vn = compat_variant_name(tname, vn)
                if rust_types:
                    fields = ", ".join(
                        f"{field_ident(f.name)}: {rust_types[f.name]}"
                        for f in c.fields if f.name in rust_types
                    )
                    lines.append(f"    /// `{c.predicate}#{c.id:08x}`")
                    lines.append(f"    {vn} {{ {fields} }},")
                else:
                    lines.append(f"    /// `{c.predicate}#{c.id:08x}`")
                    lines.append(f"    {vn},")
            if compat_global and tname in FALLTHROUGH_TYPES:
                lines.append("    /// Constructor not recognized by this library version.")
                lines.append("    Other { constructor: u32 },")
            if compat_global and tname in EXTRA_VARIANTS:
                vname, doc = EXTRA_VARIANTS[tname]
                lines.append(f"    /// {doc}")
                lines.append(f"    {vname},")
            lines.append("}")
            lines.append("")
            lines.append(f"impl {rust_t} {{")
            lines.append("    pub fn read_from(r: &mut TLReader) -> Result<Self> {")
            lines.append("        let ctor = r.read_u32()?;")
            lines.append("        match ctor {")
            # Wire-verified ctor re-issues (CTOR_ALIASES): production DCs
            # answer in the negotiated layer's dialect, where a ctor may
            # carry a different id than the fetched schema. Each alias
            # merges into its variant's match arm (same field reads).
            aliases = CTOR_ALIASES.get(tname, {})
            for c, vn, stmts, rust_types in variants:
                head = f"            {snake(c.predicate.replace(".", "_")).upper()}_ID"
                for alias_id, alias_pred in aliases.items():
                    if alias_pred == c.predicate:
                        head = (f"            {alias_id:#010x} | "
                                f"{snake(c.predicate.replace(".", "_")).upper()}_ID")
                        break
                lines.append(head + " => {")
                lines.extend(stmts)
                args = ", ".join(
                    field_ident(f.name) for f in c.fields if f.name in rust_types
                )
                lines.append(f"                Ok({rust_t}::{vn} {{ {args} }})")
                lines.append("            }")
            if compat_global and tname in FALLTHROUGH_TYPES:
                lines.append("            other => Ok("
                             f"{rust_t}::Other {{ constructor: other }}),")
            else:
                lines.append("            other => Err(Error::Serialization(format!(")
                lines.append(f'                "unknown {tname} constructor {{other:#x}}"')
                lines.append("            ))),")
            lines.append("        }")
            lines.append("    }")
            lines.append("}")
            lines.append("")
    if include_functions:
        lines.append("// ===========================================================================")
        lines.append("// Functions (request builders)")
        lines.append("// ===========================================================================")
        lines.append("")
        map_lines = gen_response_map(fn_ctors, types_by_name)
        if map_lines:
            lines.extend(map_lines)
        for c in fn_ctors:
            fl = gen_function(c)
            if fl is None:
                lines.append(f"// TODO function {c.predicate}: unsupported argument type")
                lines.append("")
                continue
            lines.extend(fl)
    out = "\n".join(lines)
    if compat_global:
        out = apply_compat_to_output(out)
    return out


def diff_consts(ctors, path):
    rx = re.compile(r"pub const ([A-Z0-9_]+): u32 = (0x[0-9a-fA-F]{8});")
    ours = {}
    for line in open(path, encoding="utf-8"):
        m = rx.search(line)
        if m:
            ours[m.group(1)] = int(m.group(2), 16)
    report = []
    for pred, c in ctors.items():
        key = snake(pred.replace(".", "_")).upper()
        for ok, ov in ours.items():
            # fuzzy: constant name contains snake parts
            pass
        # exact-style check: constant whose name equals snake upper
        if key in ours and ours[key] != c.id:
            report.append(f"  {pred}: schema 0x{c.id:08x} vs ours {key} 0x{ours[key]:08x}")
    return report



# ===========================================================================
# Compat profiles: emit curated-shaped types matching the hand-written API
# (newtype ids, curated variant names) while the generated parsers remain
# the single source of truth for the wire format.
# ===========================================================================

# id-field -> newtype wrap, applied per generated type when the field is
# the type's identity column.
NEWTYPE_FIELDS = {
    "user_id": "UserId",
    "chat_id": "ChatId",
    "channel_id": "ChannelId",
    "access_hash": "AccessHash",
}

# Generated type -> which newtype its plain `id: i64` field carries.
ID_NEWTYPE_BY_TYPE = {
    "User": "UserId", "UserEmpty": "UserId",
    "Chat": "ChatId", "ChatEmpty": "ChatId", "ChatForbidden": "ChatId",
    "Channel": "ChannelId", "ChannelForbidden": "ChannelId",
    "Photo": "PhotoId", "PhotoEmpty": "PhotoId",
    "Document": "DocumentId", "DocumentEmpty": "DocumentId",
    "MessageEmpty": "MsgId",
}

# Variant renames: generated type -> {generated variant: curated name}.
VARIANT_RENAMES = {
    "User": {"UserEmpty": "Empty"},
    "Chat": {"ChatEmpty": "Empty", "ChatForbidden": "Forbidden"},
    "Peer": {"PeerUser": "User", "PeerChat": "Chat", "PeerChannel": "Channel"},
    "InputPeer": {"InputPeerSelf": "Self_", "InputPeerUser": "User",
                   "InputPeerChat": "Chat", "InputPeerChannel": "Channel",
                   "InputPeerUserFromMessage": "UserFromMessage",
                   "InputPeerChannelFromMessage": "ChannelFromMessage"},
    "InputUser": {"InputUser": "User", "InputUserSelf": "Self_"},
    "InputChannel": {"InputChannel": "Channel"},
    "UserStatus": {"UserStatusEmpty": "Empty", "UserStatusOnline": "Online",
                    "UserStatusOffline": "Offline",
                    "UserStatusRecently": "Recently",
                    "UserStatusLastWeek": "LastWeek",
                    "UserStatusLastMonth": "LastMonth"},
    "UserProfilePhoto": {"UserProfilePhoto": "Photo",
                          "UserProfilePhotoEmpty": "Empty"},
    "Photo": {"Photo": "Photo", "PhotoEmpty": "Empty"},
    "Document": {"Document": "Document", "DocumentEmpty": "Empty"},
    "Updates": {"Updates": "Updates", "UpdateShort": "UpdateShort",
                 "UpdatesCombined": "UpdatesCombined",
                 "UpdateShortSentMessage": "UpdateShortSentMessage"},
    # Curated Update names: schema predicate minus the `update` prefix,
    # Pascal-cased (updateNewMessage -> NewMessage). Expressed as a
    # transform instead of a 200-entry table (see compat_variant_name).
    "Update": {
        "MessageId": "MessageID",
    },   # remainder via strip_prefix
    "MessageMedia": {
        "MessageMediaEmpty": "None",
        "MessageMediaPhoto": "Photo",
        "MessageMediaGeo": "Geo",
        "MessageMediaContact": "Contact",
        "MessageMediaDocument": "Document",
        "MessageMediaWebPage": "WebPage",
        "MessageMediaGame": "Game",
        "MessageMediaPoll": "Poll",
        "MessageMediaDice": "Dice",
        "MessageMediaVenue": "Venue",
        "MessageMediaGeoLive": "GeoLive",
        "MessageMediaUnsupported": "Unsupported",
    },
    "MessageAction": {
        "MessageActionEmpty": "Empty",
    },
    "ReplyMarkup": {
        "ReplyKeyboardHide": "None",
        "ReplyKeyboardForceReply": "ForceReply",
        "ReplyInlineMarkup": "InlineKeyboard",
        "ReplyKeyboardMarkup": "ReplyKeyboard",
    },
    "KeyboardButton": {
        "KeyboardButton": "Text",
        "KeyboardButtonUrl": "Url",
        "KeyboardButtonCallback": "Callback",
    },
    "InputFile": {
        "InputFile": "Id",
        "InputFileBig": "Big",
    },
    "InputDocument": {
        "InputDocument": "Document",
        "InputDocumentEmpty": "Empty",
    },
    "Message": {
        "MessageEmpty": "Empty",
        "MessageService": "Service",
        "Message": "Message",
    },
}

# Predicates whose variant name drops a type-prefix word and Pascal-cases
# the remainder (updateNewMessage -> NewMessage).
# Unions that get a synthetic `Other { constructor: u32 }` variant for
# unknown-ctor fallthrough instead of erroring.
FALLTHROUGH_TYPES = {"Update", "MessageAction", "MessageMedia"}

# Canonical home for shared dependency types when emitting per-domain
# modules. Types not listed here are owned by the domain whose seed
# closure contains them; anything else falls to the emitting domain.
CANONICAL_OWNERS = {
    "Peer": "peer",
    "InputPeer": "input", "InputUser": "input", "InputChannel": "input",
    "InputFile": "input", "InputDocument": "input",
    "InputPhoto": "input", "InputDocument": "input",
    "User": "user", "UserStatus": "user", "UserProfilePhoto": "user",
    "Chat": "chat", "ChatFull": "chat",
    "ChatAdminRights": "chat", "ChatBannedRights": "chat",
    "ChatParticipant": "chat", "ChatParticipants": "chat",
    "Photo": "photo", "PhotoSize": "photo", "Document": "photo",
    "WebDocument": "photo", "GeoPoint": "photo",
    "Message": "message", "MessageEmpty": "message",
    "MessageService": "message", "MessageMedia": "message",
    "MessageEntity": "message", "MessageAction": "message",
    "MessageReplyHeader": "message", "MessageFwdHeader": "message",
    "MessageRange": "message", "ReplyMarkup": "reply_markup",
    "KeyboardButton": "reply_markup",
    "Updates": "updates", "Update": "updates",
}

type_owner_global = {}  # tname -> domain, computed by the driver

# Unions that get a client-side sentinel unit variant (never produced by
# read_from; hand-written helpers construct it). Maps type -> variant.
EXTRA_VARIANTS = {"Peer": ("None", "Client-side sentinel — not a wire value.")}

# Domain modules (mirroring the hand-written layout). Each entry lists
# the seed RESULT TYPES; the generator pulls in every constructor of a
# seeded type plus its transitive dependencies.
DOMAIN_MODULES = {
    "peer": ["Peer"],
    "input": ["InputPeer", "InputUser", "InputChannel", "InputFile",
               "InputDocument", "InputPhoto", "InputGeoPoint",
               "InputContact", "InputStickerSet", "InputStickerSetItem",
               "InputDialogPeer", "InputStorePaymentPurpose",
               "InputWebFileLocation", "InputPhotoEmpty",
               "InputGeoPointEmpty", "InputDialog"],
    "user": ["User", "UserStatus", "UserProfilePhoto"],
    "chat": ["Chat", "ChatFull", "ChatAdminRights", "ChatBannedRights",
              "ChatParticipant", "ChatParticipants", "ExportedChatInvite"],
    "photo": ["Photo", "PhotoSize", "Document", "WebDocument", "GeoPoint"],
    "message": ["Message", "MessageEmpty", "MessageService",
                 "MessageAction", "MessageMedia", "MessageEntity",
                 "MessageReplyHeader", "MessageFwdHeader",
                 "MessageRange", "DialogFilter",
                 "messages.InvitedUsers", "MissingInvitee",
                 "MessagesMessages", "MessagesMessagesSlice",
                 "MessagesChannelMessages", "MessagesMessagesNotModified",
                 "MessagesFoundMessages", "MessagesMessagesSlice"],
    "dialog": ["Dialog", "DialogFolder", "TopPeer", "TopPeerCategory",
                "TopPeerCategoryPeers", "MessagesDialogs",
                "MessagesDialogsSlice", "MessagesMessages",
                "MessagesMessagesSlice", "MessagesChannelMessages",
                "UpdatesState", "UpdatesDifference",
                "UpdatesDifferenceSlice", "UpdatesDifferenceEmpty"],
    "reply_markup": ["ReplyMarkup", "KeyboardButton"],
    "updates": ["Updates", "Update", "UpdatesState",
                 "UpdatesDifference", "UpdatesDifferenceSlice",
                 "UpdatesDifferenceEmpty", "UpdatesChannelDifference",
                 "UpdatesChannelDifferenceEmpty",
                 "UpdatesChannelDifferenceTooLong"],
}

STRIP_PREFIX = {
    "Update": "update",
    "MessageAction": "messageAction",
    "MessageEntity": "messageEntity",
}


def pascal_stripped(prefix, predicate):
    """Strip the Pascal-cased type prefix from an already-Pascal variant
    name: 'UpdateMessageID' -update-> 'MessageID'. No-ops (returns the
    input) when the prefix does not actually match."""
    pp = pascal(prefix)
    if predicate.startswith(pp) and len(predicate) > len(pp):
        return predicate[len(pp):]
    return predicate

# Generated types whose `id` field wraps into MsgId inside variants of
# these unions (update* ctors carry message ids as int).
MSGID_ID_TYPES = {"UpdateMessageID"}
# Predicates whose `id: int` field carries a message id.
MSGID_ID_CTOR = {"updateMessageID"}


def compat_variant_name(tname, generated_name):
    renames = VARIANT_RENAMES.get(tname, {})
    if generated_name in renames:
        return renames[generated_name]
    prefix = STRIP_PREFIX.get(tname)
    if prefix:
        return pascal_stripped(prefix, generated_name)
    return generated_name


def compat_field_type(tname, field_name, rust_t):
    """Wrap a field's rust type per the newtype profile. Returns the new
    rust type or None to keep as-is."""
    nt = NEWTYPE_FIELDS.get(field_name)
    if nt:
        if rust_t == "i64":
            return nt
        if rust_t == "Option<i64>":
            return f"Option<{nt}>"
        return None
    if field_name == "id" and rust_t == "i64":
        nt = ID_NEWTYPE_BY_TYPE.get(tname)
        if nt:
            return nt
    if field_name == "id" and rust_t == "i32" and tname in MSGID_ID_TYPES:
        return "MsgId"
    return None


def apply_compat_to_output(text):
    """Post-process generated Rust source: rewrite variant/field types per
    the compat profile. Operates on the serialized output of gen_output."""
    # 1. Field wraps inside enum variant definitions and struct fields.
    #    Match `field: Type` occurrences in variant single-lines and
    #    struct bodies. Precision comes from the explicit field list.
    def wrap_fields_in_variant(m):
        body = m.group(0)
        # split variant name from fields
        return body

    # Strategy: process enum variant lines and struct field lines with a
    # targeted regex per known field name, tracking the enclosing type.
    out_lines = []
    current_type = None   # generated type name (e.g. User)
    in_enum = False
    in_impl_read = False
    for line in text.split("\n"):
        m = re.match(r"pub (?:enum|struct) (\w+)", line)
        if m:
            current_type = m.group(1)
            in_enum = line.startswith("pub enum")
            out_lines.append(line)
            continue
        if line.startswith("impl "):
            in_impl_read = True
            out_lines.append(line)
            continue
        # Ok(Type::Variant { ... }) arms inside read_from bodies
        om = re.match(r"\s+Ok\((\w+)::(\w+) \{(.*?)\}\)\s*$", line)
        if om and current_type:
            oty, ovar, args = om.groups()
            new_var = compat_variant_name(oty, ovar)
            line = re.sub(
                r"Ok\(\w+::\w+ \{",
                f"Ok({oty}::{new_var} {{",
                line,
            )
            out_lines.append(line)
            continue
        # Variant lines: `    Name { fields },` inside enums
        if in_enum and current_type:
            vm = re.match(r"(\s+)(\w+) \{ (.*) \},\s*$", line)
            if vm:
                pad, vname, fields = vm.groups()
                vname = compat_variant_name(current_type, vname)
                new_fields = []
                for f in fields.split(", "):
                    if ":" in f:
                        fname, ftype = f.split(": ", 1)
                        nt = compat_field_type(current_type, fname, ftype)
                        if nt:
                            f = f"{fname}: {nt}"
                        # Option<i64> id fields keep Option; constructor
                        # exprs handled below in read_from rewrites.
                    new_fields.append(f)
                line = f"{pad}{vname} {{ {', '.join(new_fields)} }},"
                out_lines.append(line)
                continue
        # Struct fields: `    pub name: Type,`
        sm = re.match(r"(\s+)pub (\w+): (.+?),\s*$", line)
        if sm and current_type and not in_impl_read:
            pad, fname, ftype = sm.groups()
            nt = compat_field_type(current_type, fname, ftype)
            if nt:
                line = f"{pad}pub {fname}: {nt},"
            out_lines.append(line)
            continue
        out_lines.append(line)
    return "\n".join(out_lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("schema", nargs="?", default=None,
                    help="path to a .tl schema (omit with --fetch)")
    ap.add_argument("--type", nargs="+", default=None,
                    help="predicates to generate")
    ap.add_argument("--all", action="store_true",
                    help="generate every non-function type in the schema")
    ap.add_argument("--fetch", action="store_true",
                    help="download the schema from an official source")
    ap.add_argument("--functions", action="store_true",
                    help="also generate request builders (implied by --all)")
    ap.add_argument("--save-schema", default=None,
                    help="with --fetch: where to cache the downloaded .tl")
    ap.add_argument("--out", default=None)
    ap.add_argument("--diff", action="store_true")
    ap.add_argument("--compat", action="store_true",
                    help="apply compat profiles: newtype ids, curated "
                         "variant names")
    ap.add_argument("--domain", default=None,
                    help="emit a domain module header comment")
    args = ap.parse_args()

    if args.fetch:
        path = fetch_schema(args.save_schema or "tools/schema.tl")
    else:
        path = args.schema
    if not path:
        ap.error("need a schema path or --fetch")

    text = open(path, encoding="utf-8").read()
    ctors = parse_tl(text)
    if args.diff:
        for line in diff_consts(ctors, "src/types/constructors.rs"):
            print(line)
        return
    if args.all:
        # Every type constructor, excluding the functions section and
        # built-in result types (Bool/True/Vector/...).
        requested = sorted(
            p for p, c in ctors.items()
            if c.type_name not in BUILTIN_RESULT_TYPES
        )
        include_functions = True
    else:
        if not args.type:
            if not args.domain:
                ap.error("need --type or --all")
        requested = args.type
        include_functions = args.functions
    global compat_global
    compat_global = args.compat
    if args.domain:
        if args.domain not in DOMAIN_MODULES:
            ap.error(f"unknown domain {args.domain}; choices: "
                     f"{', '.join(sorted(DOMAIN_MODULES))}")
        types_by_name = {}
        for c in ctors.values():
            types_by_name.setdefault(c.type_name, []).append(c)
        # Global ownership: every type in ANY domain's seed closure is
        # owned by the first domain whose closure contains it, unless a
        # canonical owner is pinned.
        for dom, seed_types in DOMAIN_MODULES.items():
            closure = set()
            for tname in seed_types:
                stack = [tname]
                while stack:
                    t = stack.pop()
                    if t in closure:
                        continue
                    closure.add(t)
                    for c in types_by_name.get(t, []):
                        for f in c.fields:
                            if f.tl_type in BUILTINS or f.tl_type == "#":
                                continue
                            if f.tl_type in types_by_name:
                                stack.append(f.tl_type)
            for t in closure:
                type_owner_global.setdefault(t, dom)
        seeds = []
        for tname in DOMAIN_MODULES[args.domain]:
            for c in types_by_name.get(tname, []):
                if getattr(c, "is_function", False):
                    continue
                seeds.append(c.predicate)
        if not seeds:
            ap.error(f"domain {args.domain}: no matching types in schema")
        out = gen_output(ctors, seeds, include_functions=False,
                         domain=args.domain)
        if args.out:
            with open(args.out, "w", encoding="utf-8", newline="\n") as f:
                f.write(out)
            print(f"wrote {args.out}")
        else:
            print(out)
        return
    out = gen_output(ctors, requested, include_functions)
    if args.out:
        with open(args.out, "w", encoding="utf-8", newline="\n") as f:
            f.write(out)
        print(f"wrote {args.out}")
    else:
        print(out)


OFFICIAL_SOURCES = [
    # TDLib's vendored schema — the most current machine copy. NOTE: it
    # tracks a layer revision of its own; production DCs answer in the
    # dialect of the layer the CLIENT negotiates (invokeWithLayer), which
    # can differ from this file for re-issued constructors. Wire-verified
    # divergences belong in CTOR_ALIASES below.
    ("tdlib/td",
     "https://raw.githubusercontent.com/tdlib/td/master/td/generate/scheme/telegram_api.tl"),
    # Telegram Desktop's api.tl — source of truth for tdesktop.
    ("telegramdesktop/tdesktop",
     "https://raw.githubusercontent.com/telegramdesktop/tdesktop/dev/Telegram/SourceFiles/mtproto/scheme/api.tl"),
]

# The published-layer marker only appears on the HTML schema page (the
# .tl downloads carry no layer footer, and the JSON has no layer field).
LAYER_PAGE_URL = "https://core.telegram.org/schema"

# Wire-verified constructor re-issues: production DCs answer in the
# dialect of the negotiated layer (API_LAYER), where some constructors
# are re-issued under different ids than the fetched schema carries.
# type -> {alias_id: variant_predicate} — the generator emits an extra
# match arm so both ids decode to the same variant.
#
# docs-layer branch: negotiate the PUBLISHED layer (223 — scraped from
# core.telegram.org/schema). tools/schema.tl is the 229 dev schema, so
# every ctor re-issued between 223 and 229 gets a 223 alias here
# (generated by tools/gen_docs_aliases.py from tl.json + schema.tl):
CTOR_ALIASES = {
    "BotCommand": {
        0xc27ac8c7: "botCommand",  # 223 id; 229 = 0x9852d6d2
    },
    "Chat": {
        0x1c32b11c: "channel",  # 223 id; 229 = 0xd49f34c6
    },
    "ChatFull": {
        0xe4e0b29d: "channelFull",  # 223 id; 229 = 0xa04e8d3a
    },
    "Dialog": {
        0xd58a08c6: "dialog",  # 223 id; 229 = 0xfc89f7f3
    },
    "DraftMessage": {
        0x96eaa5eb: "draftMessage",  # 223 id; 229 = 0x60fe3294
    },
    "ForumTopic": {
        0xcdff0eca: "forumTopic",  # 223 id; 229 = 0xfcdad815
    },
    "InputInvoice": {
        0xc39f5324: "inputInvoiceStarGiftResale",  # 223 id; 229 = 0xe9b0c658
    },
    "InputMedia": {
        0xb3ba0635: "inputMediaPhoto",  # 223 id; 229 = 0xe3af4434
        0x0f94e5f1: "inputMediaPoll",  # 223 id; 229 = 0x883a4108
        0x1e287d04: "inputMediaUploadedPhoto",  # 223 id; 229 = 0x7d8375da
    },
    "InputReplyTo": {
        0x869fbe10: "inputReplyToMessage",  # 223 id; 229 = 0x3bd4b7c2
    },
    "InputStorePaymentPurpose": {
        0x9bb2636d: "inputStorePaymentAuthCode",  # 223 id; 229 = 0x3fc18057
    },
    "KeyboardButton": {
        0x7d170cff: "keyboardButton",  # 223 id; 229 = 0x2f67a72f
    },
    "Message": {
        0x3ae56482: "message",  # 223 id; 229 = 0x7600b9d3
    },
    "MessageAction": {
        0xe6c31522: "messageActionStarGiftUnique",  # 223 id; 229 = 0x7e1c1187
    },
    "MessageMedia": {
        0x695150d7: "messageMediaPhoto",  # 223 id; 229 = 0xe216eb63
        0x4bd6e798: "messageMediaPoll",  # 223 id; 229 = 0x773f4e66
    },
    "MessageReplyHeader": {
        0x6917560b: "messageReplyHeader",  # 223 id; 229 = 0x1b97dd66
    },
    "PageBlock": {
        0x263d7c26: "pageBlockBlockquote",  # 223 id; 229 = 0x66d1670b
        0x9a8ae1e1: "pageBlockOrderedList",  # 223 id; 229 = 0x1fd6f6c1
    },
    "PageListItem": {
        0x25e073fc: "pageListItemBlocks",  # 223 id; 229 = 0x63ca67aa
        0xb92fb6cd: "pageListItemText",  # 223 id; 229 = 0x2f58683c
    },
    "PageListOrderedItem": {
        0x98dd8936: "pageListOrderedItemBlocks",  # 223 id; 229 = 0x8ff2d5f0
        0x5e068047: "pageListOrderedItemText",  # 223 id; 229 = 0x15031189
    },
    "Poll": {
        0x58747131: "poll",  # 223 id; 229 = 0x966e2dbf
    },
    "PollAnswer": {
        0xff16e2ca: "pollAnswer",  # 223 id; 229 = 0x4b7d786a
    },
    "PollAnswerVoters": {
        0x3b6ddad2: "pollAnswerVoters",  # 223 id; 229 = 0x3645230a
    },
    "PollResults": {
        0x7adf2420: "pollResults",  # 223 id; 229 = 0xba7bb15e
    },
    "ReactionsNotifySettings": {
        0x56e34970: "reactionsNotifySettings",  # 223 id; 229 = 0x71e4ea58
    },
    "ReplyMarkup": {
        0x48a30254: "replyInlineMarkup",  # 223 id; 229 = 0xb2b15770
    },
    "SendMessageAction": {
        0x376d975c: "sendMessageTextDraftAction",  # 223 id; 229 = 0x3630b85a
    },
    "SentCode": {
        0xe0955a3c: "auth.sentCodePaymentRequired",  # 223 id; 229 = 0xf8827ebf
    },
    "StoryItem": {
        0xedf164f1: "storyItem",  # 223 id; 229 = 0x16a4b93c
    },
    "Update": {
        0x11dfa986: "updateBotChatInviteRequester",  # 223 id; 229 = 0x7cb34d79
        0xaca1657b: "updateMessagePoll",  # 223 id; 229 = 0xd64c522b
        0x24f40e77: "updateMessagePollVote",  # 223 id; 229 = 0x7699f014
    },
    "UrlAuthResult": {
        0xf8f8eb1e: "urlAuthResultRequest",  # 223 id; 229 = 0x3cd623ec
    },
    "User": {
        0x31774388: "user",  # 223 id; 229 = 0xb1b8cc83
    },
}


def fetch_published_layer():
    """Scrape the published layer number from the HTML schema page.
    Returns None when the page does not carry it."""
    import re
    import urllib.request
    try:
        req = urllib.request.Request(LAYER_PAGE_URL,
                                     headers={"User-Agent": "mtprsto-gentl"})
        html = urllib.request.urlopen(req, timeout=30).read().decode("utf-8")
        m = re.search(r"[Ll]ayer\s*(\d+)", html)
        return int(m.group(1)) if m else None
    except Exception as e:  # noqa: BLE001 — best-effort metadata
        print(f"  layer scrape failed: {e}", file=sys.stderr)
        return None


def fetch_schema(dest):
    """Download the API schema from an official source, trying each in
    order. Also scrapes the published layer id (the .tl sources carry no
    layer marker; only the HTML schema page does) and records both in a
    sidecar meta file next to the schema."""
    import json
    import datetime
    import urllib.request
    for name, url in OFFICIAL_SOURCES:
        try:
            print(f"fetching {name}: {url} ...", file=sys.stderr)
            req = urllib.request.Request(url, headers={"User-Agent": "mtprsto-gentl"})
            data = urllib.request.urlopen(req, timeout=30).read()
            if len(data) < 10_000:
                raise RuntimeError(f"response too small ({len(data)} bytes)")
            text = data.decode("utf-8")
            if "Layer" not in text and "//" not in text:
                raise RuntimeError("response does not look like a .tl schema")
            with open(dest, "w", encoding="utf-8", newline="\n") as f:
                f.write(text)
            layer = fetch_published_layer()
            meta = {
                "source": name,
                "url": url,
                "fetched_utc": datetime.datetime.now(
                    datetime.timezone.utc).isoformat(timespec="seconds"),
                "published_layer": layer,
                "bytes": len(data),
            }
            with open(dest + ".meta.json", "w", encoding="utf-8") as f:
                json.dump(meta, f, indent=2)
                f.write("\n")
            print(f"saved {len(text)} bytes to {dest}", file=sys.stderr)
            print(f"published layer: {layer}", file=sys.stderr)
            return dest
        except Exception as e:  # noqa: BLE001 — try the next source
            print(f"  failed: {e}", file=sys.stderr)
    print("all official sources failed", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
