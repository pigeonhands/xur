# Xur

Xur is a small, expression-oriented programming language with Rust-like
syntax, modules that are ordinary values, and explicit control over data
layout.

> **Status:** early development. The lexer and the `xur debug tokens` command
> work; the parser, AST and runtime are still to come.

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

### Functions and generics

```rust
pub fn add(a: u32, b: u32) -> u32 {
    a + b // the last expression is the return value
}

fn print_name<T>(s: T) {
    std::io::print("Struct name: {}", std::any::type_name::<T>());
}

print_name::<ZeroPacked>();
```

### `if` and `match` are expressions

```rust
let parity = if i % 2 == 0 { "even" } else { "odd" };

enum Shape {
    Circle(f64)
    Rect(f64, f64)
}

fn area(shape: Shape) -> f64 {
    match shape {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rect(w, h) => w * h,
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

```console
$ cargo run -- debug tokens example/math.xur
1:1      pub        pub
1:5      enum       enum
1:10     <ident>    MathError
1:20     {          {
...
```

Lex errors are shown as annotated source snippets, and the command exits
non-zero:

```console
$ cargo run -- debug tokens bad.xur
Error: unterminated string literal
   ╭─[ bad.xur:2:9 ]
   │
 2 │ let s = "oops
   │         ──┬──
   │           ╰──── missing closing `"`
───╯
error: could not lex `bad.xur` due to 1 error(s)
```

## Workspace layout

| Crate                 | Purpose                                       |
| --------------------- | --------------------------------------------- |
| `crates/xur`          | The `xur` command-line tool                   |
| `crates/xur-parser`   | Lexer (and later the parser)                  |
| `crates/xur-ast`      | Syntax tree types (planned)                   |
| `crates/xur-runtime`  | Runtime (planned)                             |

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## License

Licensed under either of MIT or Apache-2.0, at your option.
