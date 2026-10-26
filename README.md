# Xur

Xur is a small, expression-oriented programming language with Rust-like
syntax, modules that are ordinary values, and explicit control over data
layout.

> **Status:** early development. Programs can be lexed, parsed and have
> their names resolved across modules (`xur debug tokens`, `xur debug ast`,
> `xur debug resolve`); type checking and the runtime are still to come.

## A taste of Xur

### Modules are values

```rust
use std;

const math = std::import("./math.xur"); // load a module

fn main() {
    let io = std::io; // bind a module to a variable
    io::println("1 + 2 = {}", math.add(1, 2));
}
```

`std::import` is an ordinary function, so the path can be computed at
runtime and the module stored like any other value:

```rust
let modules = [math, std::import(plugin_path)];
for m in modules {
    io::println("2 + 3 = {}", m.add(2, 3));
}
```

### Functions and generics

```rust
pub fn add(a: u32, b: u32) -> u32 {
    a + b // the last expression is the return value
}

fn print_name<T>() {
    std::io::println("Struct name: {}", std::any::type_name::<T>());
}

print_name::<ZeroPacked>();
```

### `if` and `match` are expressions, and `impl` adds methods

```rust
let parity = if i % 2 == 0 { "even" } else { "odd" };

enum Shape {
    Circle(f64)
    Rect(f64, f64)
}

impl Shape {
    // a method, called on a value: `shape.area()`
    pub fn area(self) -> f64 {
        match self {
            Shape::Circle(r) => 3.14159 * r * r,
            Shape::Rect(w, h) => w * h,
        }
    }

    // an associated function, called on the type: `Shape::square(2.0)`
    pub fn square(side: f64) -> Self {
        Shape::Rect(side, side)
    }
}
```

### Errors are values: `Result` and `?`

`Result` is an ordinary enum, `enum Result<T, E> { Ok(T) Err(E) }`. The std
prelude provides it.

```rust
pub enum MathError {
    DivideByZero
    Overflow
}

pub fn div(a: u32, b: u32) -> Result<u32, MathError> {
    if b == 0 {
        Err(MathError::DivideByZero)
    } else {
        Ok(a / b)
    }
}

// `?` returns early with the `Err`, otherwise unwraps the `Ok` value
fn average(total: u32, count: u32) -> Result<u32, MathError> {
    let avg = div(total, count)?;
    Ok(avg)
}

match average(10, 0) {
    Ok(avg) => io::println("average = {}", avg),
    Err(MathError::DivideByZero) => io::println("cannot average zero items"),
    Err(_) => io::println("something else went wrong"),
}
```

### Closures

```rust
let double = |x: u32| x * 2;

let offset = 10;
let shift = |x| {
    let y = double(x);
    y + offset // captures `offset`
};
```

### Control over memory layout

```rust
#[repr(C)] // C ABI alignment & padding
struct CPacked {
    a: usize
    b: u32
}

#[repr(packed)] // no padding, 1-byte alignment
struct ZeroPacked {
    a: u32
    b: u32
}

#[repr(align(4))]
#[endian(Little)]
struct Aligned {
    a: u32
    b: u32
    #[field(offset=100)] c: u32 // 100 bytes from the start of the struct
}
```

There are complete programs in [`example/`](example).

## Using the CLI

`xur debug` shows the output of individual compiler stages. `tokens` prints
what the lexer produces:

```console
$ cargo run -- debug tokens example/math.xur
1:1      pub        pub
1:5      enum       enum
1:10     <ident>    MathError
1:20     {          {
...
```

`ast` prints the syntax tree (spans are byte ranges):

```console
$ cargo run -- debug ast add.xur
Module {
    items: [
        Item {
            attrs: [],
            vis: Public(
                0..3,
            ),
            kind: Fn(
                Fn {
                    ident: "add"@7..10,
...
```

`resolve` loads the whole program, following `std::import`, and shows what
every name refers to:

```console
$ cargo run -- debug resolve example/main.xur
...
example/main.xur:50:9  io                           new local
example/main.xur:50:14 std::io                      module std::io
example/main.xur:52:5  io::println                  builtin function std::io::println
...
example/main.xur:69:13 math::MathError::DivideByZero variant math::MathError::DivideByZero
...
```

Errors are shown as annotated source snippets and the command exits
non-zero. Like rustc, the parser recovers from errors, so every mistake in a
file is reported in one run:

```console
$ cargo run -- debug ast bad.xur
Error: expected one of `(`, `.`, `;`, `?`, `[`, or an operator, found keyword `let`
   ╭─[ bad.xur:2:14 ]
   │
 2 │     let x = 1
   │              │
   │              ╰─ expected one of 6 possible tokens
 3 │     let y = x;
   │     ─┬─
   │      ╰─── unexpected token
───╯
error: could not parse `bad.xur` due to 1 error(s)
```

## Workspace layout

The compiler is structured after rustc, whose crates these mirror:

| Crate                 | Purpose                                                  | rustc analogue  |
| --------------------- | -------------------------------------------------------- | --------------- |
| `crates/xur`          | The `xur` command-line tool                              | `rustc_driver`  |
| `crates/xur-parser`   | Lexer and recursive-descent parser                       | `rustc_parse`   |
| `crates/xur-resolve`  | Module loading (`std::import`) and name resolution       | `rustc_resolve` |
| `crates/xur-ast`      | Syntax tree types                                        | `rustc_ast`     |
| `crates/xur-errors`   | Diagnostics and their rendering                          | `rustc_errors`  |
| `crates/xur-span`     | Source map, spans and interned symbols                   | `rustc_span`    |
| `crates/xur-runtime`  | Runtime (planned)                                        |                 |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## License

Licensed under either of MIT or Apache-2.0, at your option.
