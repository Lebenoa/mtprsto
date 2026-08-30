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
        # Box every nested object: keeps recursive unions (InputPeer,
        # RichText, ...) finite-sized.
        rust_t = rust_type_name(t)
        return [f"{pad}let {n} = Box::new({rust_t}::read_from(r)?);"], f"Box<{rust_t}>"
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
    return stmts, rust_types


def gen_output(ctors, requested, include_functions=False):
    global types_by_name_global, generatable_global, writable_global
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

    lines = []
    lines.append("//! GENERATED by tools/gentl.py from a Telegram API .tl schema.")
    lines.append("//! Field order and flags come straight from the schema — do not hand-edit.")
    lines.append("//! Unread/unused fields keep their reads so the stream stays aligned.")
    lines.append("#![allow(dead_code, unused_variables, unused_mut)]")
    lines.append("#![allow(clippy::large_enum_variant)]")
    lines.append("")
    lines.append("use crate::error::{Error, Result};")
    lines.append("use crate::serialize::TLReader;")
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
            body = gen_ctor(c, ctors, types_by_name, generatable)
            if body is None:
                lines.append(f"// TODO {c.predicate}: contains a type this generator does not know")
                lines.append("")
                continue
            stmts, rust_types = body
            lines.append(f"/// `{c.predicate}#{c.id:08x} = {tname}`")
            lines.append("#[derive(Debug, Clone)]")
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
            write_body = gen_write_to(c, rust_types)
            if write_body is not None:
                lines.append("    /// Serialize in schema field order (flags auto-computed).")
                lines.append("    pub fn write_to(&self, w: &mut crate::serialize::TLWriter) {")
                lines.extend(write_body)
                lines.append("    }")
            lines.append("}")
            lines.append("")
        else:
            supported = [c for c in group if gen_ctor(c, ctors, types_by_name, generatable) is not None]
            if not supported:
                lines.append(f"// TODO union {tname}: no constructor is generatable "
                             f"({', '.join(c.predicate for c in group)})")
                lines.append("")
                continue
            lines.append(f"/// Union `{tname}` ({len(group)} constructors).")
            lines.append("#[derive(Debug, Clone)]")
            lines.append(f"pub enum {rust_t} {{")
            variants = []
            for c in group:
                body = gen_ctor(c, ctors, types_by_name, generatable)
                if body is None:
                    continue
                stmts, rust_types = body
                vn = pascal(c.predicate.replace(".", "_"))
                variants.append((c, vn, stmts, rust_types))
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
            lines.append("}")
            lines.append("")
            lines.append(f"impl {rust_t} {{")
            lines.append("    pub fn read_from(r: &mut TLReader) -> Result<Self> {")
            lines.append("        let ctor = r.read_u32()?;")
            lines.append("        match ctor {")
            for c, vn, stmts, rust_types in variants:
                lines.append(f"            {snake(c.predicate.replace(".", "_")).upper()}_ID => {{")
                lines.extend(stmts)
                args = ", ".join(
                    field_ident(f.name) for f in c.fields if f.name in rust_types
                )
                lines.append(f"                Ok({rust_t}::{vn} {{ {args} }})")
                lines.append("            }")
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
        for c in fn_ctors:
            fl = gen_function(c)
            if fl is None:
                lines.append(f"// TODO function {c.predicate}: unsupported argument type")
                lines.append("")
                continue
            lines.extend(fl)
    return "\n".join(lines)


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
            ap.error("need --type or --all")
        requested = args.type
        include_functions = args.functions
    out = gen_output(ctors, requested, include_functions)
    if args.out:
        with open(args.out, "w", encoding="utf-8", newline="\n") as f:
            f.write(out)
        print(f"wrote {args.out}")
    else:
        print(out)


OFFICIAL_SOURCES = [
    # TDLib's vendored schema — the most current machine copy.
    ("tdlib/td",
     "https://raw.githubusercontent.com/tdlib/td/master/td/generate/scheme/telegram_api.tl"),
    # Telegram Desktop's api.tl — source of truth for tdesktop.
    ("telegramdesktop/tdesktop",
     "https://raw.githubusercontent.com/telegramdesktop/tdesktop/dev/Telegram/SourceFiles/mtproto/scheme/api.tl"),
    # The official published page (lags on ctor re-issues but is canonical).
    ("core.telegram.org",
     "https://core.telegram.org/schema/tl"),
]


def fetch_schema(dest):
    """Download the API schema from an official source, trying each in
    order. Returns the path it was saved to."""
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
            print(f"saved {len(text)} bytes to {dest}", file=sys.stderr)
            return dest
        except Exception as e:  # noqa: BLE001 — try the next source
            print(f"  failed: {e}", file=sys.stderr)
    print("all official sources failed", file=sys.stderr)
    sys.exit(1)


if __name__ == "__main__":
    main()
