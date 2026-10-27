# Architecture

This is how a `.xur` file goes from source code to running. The stages,
and the crates that implement them, follow rustc's.

```text
                  xur-parser                 xur-resolve          xur-bytecode       xur-vm
               ┌──────────────┐        ┌────────────────────┐   ┌─────────────┐   ┌──────────┐
 greet.xur ──▶ │ lexer→parser │ ─AST─▶ │ load modules,      │ ─▶│ lower to    │ ─▶│ interpret│ ─▶ output
               └──────────────┘        │ resolve every name │   │ bytecode    │   │ bytecode │
                                       └────────────────────┘   └─────────────┘   └──────────┘
                     ▲                          ▲                                       ▲
              xur-span, xur-errors        native modules (xur-std) via xur-runtime ─────┘
```

We follow this program through every stage; each listing is the real
output of the `xur debug` command that shows it.

```rust
fn greet(name: str) -> str {
    "Hello, " + name
}

fn main() {
    let names = ["Ada", "Grace"];
    for name in names {
        std::io::println("{}", greet(name));
    }
}
```

## 0. Source files and spans (`xur-span`)

Every loaded file is added to a `SourceMap`, which lays files out one after
another in a single space of byte positions. A `Span` is just a byte range
in that space, so it identifies its file as well as its location, and
line/column positions are only computed when a message needs them.

Names are interned: a `Symbol` is a small index into a global table of
strings, so names are `Copy` and cheap to compare. An `Ident` is a symbol
with the span it was written at.

## 1. Lexing (`xur-parser::lexer`)

The lexer turns text into tokens that borrow from the source:

```console
$ xur debug tokens greet.xur
1:1      fn         fn
1:4      <ident>    greet
1:9      (          (
1:10     <ident>    name
1:14     :          :
1:16     <ident>    str
1:19     )          )
1:21     ->         ->
1:24     <ident>    str
1:28     {          {
2:5      <string>   "Hello, "
2:15     +          +
2:17     <ident>    name
3:1      }          }
...
```

It never stops at an error: an unknown character is reported and skipped,
an unterminated string still becomes a string. Literals keep their source
text; turning them into values (and reporting overflow or bad escapes) is
the parser's job, as in rustc.

## 2. Parsing (`xur-parser::parser`, `xur-ast`)

A recursive-descent parser builds the abstract syntax tree. Expressions are
parsed by precedence climbing over an operator table. Errors read like
rustc's (`expected one of ``,`` or ``)``, found ``;```), and the parser
recovers at statement and item boundaries so one mistake doesn't hide the
rest.

Every node pairs a `kind` with its span. Nodes that define or use names
get a `NodeId`, so later stages can record facts about them in side tables
instead of mutating the tree:

```console
$ xur debug ast greet.xur
Module {
    items: [
        Item {
            id: #5,
            attrs: [],
            vis: Inherited,
            kind: Fn(
                Fn {
                    ident: "greet"@3..8,
                    ...
                    decl: FnDecl {
                        inputs: [
                            Param {
                                pat: Pat {
                                    id: #1,
                                    kind: Ident(
                                        "name"@9..13,
                                    ),
                                    ...
                    body: Block {
                        stmts: [
                            Stmt {
                                kind: Expr(
                                    Expr {
                                        kind: Binary(
                                            BinOp {
                                                kind: Add,
                                            ...
```

Parsed modules are allocated for the life of the process, as rustc
arena-allocates its ASTs, so later stages can hold references into them.

## 3. Loading and name resolution (`xur-resolve`)

A `Program` starts from the root file. The loader finds every
`std::import("path")` with a literal path and loads those files too
(relative to the importing file, each once, so cycles are fine). Together
with the prelude, written in Xur, they form the program's modules.

Resolution then works out what every name refers to, in phases:

1. **Native modules** like `std` are declared from descriptions given by
   the runtime's registry (see below). Lazy ones stay empty until a path
   looks inside them.
2. **Items** of every module are declared in its scope, with separate type
   and value namespaces. Each definition gets a `DefId`.
3. **`use` declarations** are resolved, repeatedly until no more make
   progress, since one import may depend on another.
4. **`impl` blocks** add their functions to their type's scope.
5. **Bodies**: every path and pattern inside items is resolved against a
   stack of local scopes ("ribs") and then the module.

The result maps `NodeId`s to what they mean:

```console
$ xur debug resolve greet.xur
greet.xur:1:10         name                         new local
greet.xur:1:16         str                          builtin type str
greet.xur:1:24         str                          builtin type str
greet.xur:2:17         name                         local name at greet.xur:1:10
greet.xur:6:9          names                        new local
greet.xur:7:9          name                         new local
greet.xur:7:17         names                        local names at greet.xur:6:9
greet.xur:8:9          std::io::println             native function std::io::println
greet.xur:8:32         greet                        function greet
greet.xur:8:38         name                         local name at greet.xur:7:9
```

Only names resolve statically. Members reached through a value, like
`math.add` on a module value or `point.x`, are looked up when the program
runs. A program can keep growing while it runs: `Program::import` loads and
resolves new modules (a `std::import` of a computed path) without
invalidating any existing `DefId` or `NodeId`.

## 4. Lowering to bytecode (`xur-bytecode`)

Functions are lowered the first time they are called. Each becomes a
control-flow graph of basic blocks over virtual registers; each block ends
in a terminator (`goto`, `branch`, `return`, `next` for loops):

```console
$ xur debug bytecode greet.xur
fn greet (captures: 0, params: 1, registers: 3)
  bb0:
    check r0: str
    r1 = "Hello, "
    r2 = r1 + r0
    check r2: str
    return r2

fn main (captures: 0, params: 0, registers: 14)
  bb0:
    r0 = "Ada"
    r1 = "Grace"
    r2 = [r0, r1]
    r3 = r2
    r4 = iter r3
    goto bb1
  bb1:
    r5 = next r4 ? bb2 : bb3
  bb2:
    r6 = ()
    r7 = native #3
    r8 = "{}"
    r9 = fn greet
    r10 = call r9(r5)
    r11 = call r7(r8, r10)
    r12 = ()
    goto bb1
  bb3:
    r13 = ()
    return r13
```

Lowering makes everything explicit:

- **Patterns** become test-and-branch sequences that bind registers.
- **`if`, `match`, loops, `&&`, `||` and `?`** become blocks and branches.
- **Closures** become separate functions whose first registers hold the
  values they captured.
- **Written types become `check` instructions.** Xur is checked at runtime
  where a type is written (`name: str` above); unwritten and generic types
  are never checked.

The registers hold dynamically typed values, but the shape of the code (a
CFG of blocks over virtual registers) is Cranelift's, so a JIT can compile
the same bytecode the interpreter runs. `xur-jit` is the start of one: it
sets up Cranelift for the host, but doesn't compile anything yet.

## 5. Running (`xur-vm`, `xur-runtime`)

The VM keeps what all threads share: the `Program` (behind a lock, since
imports extend it), the registry, the cache of lowered functions, constant
values and stdout. Each thread runs an interpreter over a function's
register file, block by block:

```console
$ xur run greet.xur
Hello, Ada
Hello, Grace
```

Values (`xur-runtime::Value`) carry what they are. Lists and native
objects are shared and mutable, everything else immutable, and all of them
are `Send + Sync` so threads can share them. `value.method()` is dispatched
on the value: `impl` functions for Xur structs and enums, registered
methods for native objects and built-in values like lists, and members for
module values. Runtime errors point at the expression that failed.

## Native code (`xur-runtime`, `xur-std`)

Rust functions and types reach Xur through a `Registry`, using
[facet](https://facet.rs) reflection:

- Any `Fn(A, B, ...) -> R` whose types implement `Facet` can be
  registered. Arguments are converted from Xur values by walking the
  type's shape with facet's `Partial`, and results back with `Peek`.
- A registered `#[derive(Facet)]` type lives in Xur as the Rust value
  itself, so methods taking `&mut self` change it in place.
- Functions that need the running program (`std::import`, `thread::spawn`)
  take a `Context`, which the VM implements.
- The registry describes its modules to the resolver by name only, and
  lazy modules are only built when a program first uses them.

`std` (`xur-std`) is written entirely this way, as an ordinary client of
the registry: `io`, `fs`, `net`, `thread`, `sync`, `math`, `any`, `reflect`, `mem`, `re`, and
methods for lists, strings and numbers.

## Diagnostics (`xur-errors`)

Every stage reports problems as a `Diag`: a message, a primary span,
labelled spans and notes. A single emitter renders them as annotated source
snippets with ariadne, in whichever file of the source map they point.
