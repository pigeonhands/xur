//! `std::process`: spawning and controlling OS child processes.
//!
//! ```text
//! let child = std::process::spawn("sleep", ["5"]).unwrap();
//! io::println("pid {}", child.pid());
//! child.kill().unwrap();
//! child.wait().unwrap();
//! ```

use std::process::Child;
use std::sync::Mutex;

use facet::Facet;
use xur_runtime::convert::into_value;
use xur_runtime::{Registry, RunResult, RuntimeError, Value};

use crate::{error_text, result};

/// A spawned child process.
#[derive(Facet)]
#[facet(opaque)]
pub struct Process(Mutex<Child>);

pub(crate) fn register(registry: &Registry) {
    registry.lazy_module("std::process", |module| {
        module
            .raw_function("spawn", |cx, args| {
                let [Value::Str(program), Value::List(args_list)] = args else {
                    return Err(RuntimeError::new(
                        "`spawn` takes a program path and a list of arguments",
                    ));
                };
                let program_args: Vec<String> = xur_runtime::lock(args_list)
                    .iter()
                    .map(|value| match value {
                        Value::Str(s) => Ok(s.to_string()),
                        other => Err(RuntimeError::new(format!(
                            "expected a string argument, found {}",
                            other.kind()
                        ))),
                    })
                    .collect::<RunResult<_>>()?;
                match std::process::Command::new(&**program)
                    .args(&program_args)
                    .spawn()
                {
                    Ok(child) => {
                        let value = into_value(cx, Process(Mutex::new(child)))?;
                        Ok(result(cx, Ok(value)))
                    }
                    Err(error) => Ok(result(cx, Err(error_text(error)))),
                }
            })
            .ty_borrowing::<Process>("Process")
            .method("pid", |p: &Process| p.0.lock().unwrap().id())
            .method_mut("kill", |p: &mut Process| {
                p.0.lock().unwrap().kill().map_err(error_text)
            })
            .method_mut("wait", |p: &mut Process| {
                p.0.lock()
                    .unwrap()
                    .wait()
                    .map(|status| status.code().unwrap_or(-1))
                    .map_err(error_text)
            })
            .method_mut("try_wait", |p: &mut Process| {
                p.0.lock()
                    .unwrap()
                    .try_wait()
                    .map(|status| status.map(|s| s.code().unwrap_or(-1)))
                    .map_err(error_text)
            });
    });
}
