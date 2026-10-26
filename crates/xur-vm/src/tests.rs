use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use xur_resolve::Program;
use xur_span::MemoryFileLoader;

use crate::Vm;

/// Captures what a program prints.
#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Runs the program made of `files` (the first is the root), returning
/// what it printed, or the runtime error.
fn run_files(files: &[(&str, &str)]) -> Result<String, String> {
    let registry = Arc::new(xur_std::registry());
    let loader = MemoryFileLoader::new(files.iter().copied());
    let (program, diags) = Program::load(
        Path::new(files[0].0),
        Box::new(loader),
        Box::new(Arc::clone(&registry)),
    )
    .unwrap();
    let messages: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(diags.is_empty(), "{messages:?}");
    let output = Output::default();
    let vm = Vm::new(program, registry, Box::new(output.clone()));
    let result = vm.run_main();
    let printed = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
    result.map(|_| printed).map_err(|error| error.message)
}

fn run(src: &str) -> Result<String, String> {
    run_files(&[("main.xur", src)])
}

#[test]
fn hello_world() {
    assert_eq!(
        run("fn main() { std::io::println(\"Hello, {}!\", \"World\"); }"),
        Ok("Hello, World!\n".into())
    );
}

#[test]
fn arithmetic_control_flow_and_recursion() {
    let src = "
        fn fib(n: i64) -> i64 { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
        fn main() {
            let parity = if 7 % 2 == 0 { \"even\" } else { \"odd\" };
            std::io::println(\"{} {} {}\", fib(15), parity, 1.5 * 2.0);
        }";
    assert_eq!(run(src), Ok("610 odd 3\n".into()));
}

#[test]
fn enums_impls_and_match() {
    let src = "
        enum Shape { Circle(f64) Rect(f64, f64) }
        impl Shape {
            fn area(self) -> f64 { match self { Shape::Circle(r) => 3.0 * r * r, Shape::Rect(w, h) => w * h } }
            fn square(side: f64) -> Self { Shape::Rect(side, side) }
        }
        fn main() {
            let shapes = [Shape::Circle(1.0), Shape::square(2.0)];
            for s in shapes { std::io::println(\"{} {}\", s, s.area()); }
        }";
    assert_eq!(run(src), Ok("Circle(1) 3\nRect(2, 2) 4\n".into()));
}

#[test]
fn closures_capture_values() {
    let src = "
        fn apply(f: _, x: i64) -> i64 { f(x) }
        fn main() {
            let offset = 10;
            let shift = |x| { let y = x * 2; y + offset };
            std::io::println(\"{}\", apply(shift, 4));
        }";
    assert_eq!(run(src), Ok("18\n".into()));
}

#[test]
fn lists_are_shared_and_mutable() {
    let src = "
        fn main() {
            let xs = [1, 2, 3];
            let alias = xs;
            alias.push(4);
            xs[0] = 10;
            let total = 0;
            for x in xs { total = total + x; }
            std::io::println(\"{} {} {}\", xs, xs.len(), total);
        }";
    assert_eq!(run(src), Ok("[10, 2, 3, 4] 4 19\n".into()));
}

#[test]
fn results_and_the_try_operator() {
    let src = "
        fn half(n: i64) -> Result<i64, str> { if n % 2 == 0 { Ok(n / 2) } else { Err(\"odd\") } }
        fn quarter(n: i64) -> Result<i64, str> { let h = half(n)?; half(h) }
        fn main() {
            std::io::println(\"{} {}\", quarter(8), quarter(6));
            match \"12\".parse_int() { Ok(n) => std::io::println(\"{}\", n + 1), Err(e) => std::io::println(\"{}\", e) }
        }";
    assert_eq!(run(src), Ok("Ok(2) Err(\"odd\")\n13\n".into()));
}

#[test]
fn loops_break_and_continue() {
    let src = "
        fn main() {
            let found = loop { break 42; };
            let n = 0;
            while true { n = n + 1; if n < 5 { continue; } break; }
            std::io::println(\"{} {}\", found, n);
        }";
    assert_eq!(run(src), Ok("42 5\n".into()));
}

#[test]
fn modules_are_values_and_imports_can_be_dynamic() {
    let files = [
        (
            "main.xur",
            "const math = std::import(\"math.xur\");
             fn main() {
                 let name = \"math.xur\";
                 let modules = [math, std::import(name)];
                 for m in modules { std::io::println(\"{}\", m.add(2, 3)); }
                 std::io::println(\"{}\", math::add(1, 1));
             }",
        ),
        ("math.xur", "pub fn add(a: i64, b: i64) -> i64 { a + b }"),
    ];
    assert_eq!(run_files(&files), Ok("5\n5\n2\n".into()));
}

#[test]
fn type_names_follow_type_arguments() {
    let src = "
        struct Marker;
        fn name_of<T>() -> str { std::any::type_name::<T>() }
        fn main() { std::io::println(\"{} {}\", name_of::<Marker>(), name_of::<u32>()); }";
    assert_eq!(run(src), Ok("Marker u32\n".into()));
}

#[test]
fn runtime_errors() {
    assert_eq!(
        run("fn main() { 1 / 0; }"),
        Err("attempt to divide by zero".into())
    );
    assert_eq!(
        run("fn main() { 1 + 1.0; }"),
        Err("cannot apply `+` to integer and float".into())
    );
    assert_eq!(
        run("fn main() { [1].nope(); }"),
        Err("no method `nope` on list".into())
    );
    assert_eq!(
        run("fn f() {}"),
        Err("the program has no `main` function".into())
    );
}

#[test]
fn the_examples_run() {
    let output = run_files(&[
        (
            "example/main.xur",
            include_str!("../../../example/main.xur"),
        ),
        (
            "example/math.xur",
            include_str!("../../../example/math.xur"),
        ),
    ]);
    insta_like(
        output,
        "Hello, World
1 + 0 = 1 (even)
1 + 1 = 2 (odd)
1 + 2 = 3 (even)
1 + 3 = 4 (odd)
1 + 4 = 5 (even)
1 + 5 = 6 (odd)
1 + 6 = 7 (even)
1 + 7 = 8 (odd)
1 + 8 = 9 (even)
1 + 9 = 10 (odd)
shift(4) = 18
cannot average zero items
2 + 3 = 5
2 + 3 = 5
area = 7
square area = 4
Struct name: ZeroPacked
",
    );
}

fn insta_like(actual: Result<String, String>, expected: &str) {
    match actual {
        Ok(actual) if actual == expected => {}
        other => panic!("expected:\n{expected}\nactual:\n{other:?}"),
    }
}

#[test]
fn option_and_result_methods_are_written_in_xur() {
    let src = "
        fn main() {
            let some = Some(2);
            let none: Option<i64> = None;
            std::io::println(\"{} {} {}\", some.is_some(), none.unwrap_or(7), some.map(|x| x * 10));
            let parsed = \"x\".parse_int();
            std::io::println(\"{} {}\", parsed.is_err(), parsed.ok());
        }";
    assert_eq!(run(src), Ok("true 7 Some(20)\ntrue None\n".into()));
}

#[test]
fn panics_stop_the_program() {
    assert_eq!(
        run("fn main() { let x: Option<i64> = None; x.unwrap(); }"),
        Err("called `unwrap` on `None`".into())
    );
    assert_eq!(
        run("fn main() { std::panic(\"bad {}\", 1); }"),
        Err("bad 1".into())
    );
}

#[test]
fn files() {
    let dir = std::env::temp_dir().join(format!("xur-fs-test-{}", std::process::id()));
    let dir = dir.display().to_string();
    let src = format!(
        "fn main() {{
            let dir = \"{dir}\";
            std::fs::create_dir_all(dir).unwrap();
            let path = dir + \"/notes.txt\";
            std::fs::write(path, \"one\\n\").unwrap();
            std::fs::append(path, \"two\\n\").unwrap();
            let file = std::fs::File::open(path).unwrap();
            std::io::println(\"{{}}\", file.read_to_string().unwrap().lines());
            std::io::println(\"{{}} {{}}\", std::fs::exists(path), std::fs::read_dir(dir).unwrap());
            std::fs::remove(path).unwrap();
            std::io::println(\"{{}}\", std::fs::read_to_string(path).is_err());
        }}"
    );
    let output = run(&src);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        output,
        Ok("[\"one\", \"two\"]\ntrue [\"notes.txt\"]\ntrue\n".into())
    );
}

#[test]
fn threads_and_channels() {
    let src = "
        fn main() {
            let (sender, receiver) = std::sync::channel();
            let workers = [];
            for i in 1..4 {
                workers.push(std::thread::spawn(|| {
                    sender.send(i * i).unwrap();
                    i
                }));
            }
            let joined = 0;
            for worker in workers { joined = joined + worker.join(); }
            let received = 0;
            for _ in 0..3 { received = received + receiver.recv().unwrap(); }
            std::io::println(\"{} {} {}\", joined, received, receiver.try_recv());
        }";
    assert_eq!(run(src), Ok("6 14 None\n".into()));
}

#[test]
fn errors_in_threads_reach_the_joiner() {
    let src = "fn main() { std::thread::spawn(|| 1 / 0).join(); }";
    assert_eq!(run(src), Err("attempt to divide by zero".into()));
}
