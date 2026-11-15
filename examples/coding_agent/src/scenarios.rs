//! A tour of coding-agent scenarios of increasing complexity.
//!
//! Each scenario sets up a fixture in a fresh workspace, gives the **real**
//! agent a task, and then **independently verifies** the result (by running the
//! produced code ourselves). No mocks — the agent must actually do the work.

use std::path::Path;
use std::process::Command;

/// One scenario: a fixture, a task, and an independent verification.
pub struct Scenario {
    /// Short id (used to run a single scenario by name).
    pub name: &'static str,
    /// One-line description shown in the tour.
    pub blurb: &'static str,
    /// Prepare the workspace before the agent runs.
    pub setup: fn(&Path) -> std::io::Result<()>,
    /// The instruction given to the agent.
    pub task: &'static str,
    /// Verify the outcome after the agent finishes. Returns (passed, detail).
    pub verify: fn(&Path) -> (bool, String),
}

/// All scenarios, ordered from simplest to most involved.
pub fn all() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "hello",
            blurb: "single file, no execution",
            setup: |_| Ok(()),
            task: "Create a file named hello.txt containing exactly the text: hello world",
            verify: |dir| {
                let got = std::fs::read_to_string(dir.join("hello.txt")).unwrap_or_default();
                let ok = got.trim() == "hello world";
                (ok, format!("hello.txt = {:?}", got.trim()))
            },
        },
        Scenario {
            name: "multifile",
            blurb: "create a 2-file program and run it",
            setup: |_| Ok(()),
            task: "Create util.py defining `def add(a, b): return a + b`. Then create app.py \
                   that imports add from util and prints add(40, 2). Run `python3 app.py` and \
                   report the output.",
            verify: |dir| {
                let (code, out, _) = run(dir, "python3", &["app.py"]);
                let ok = code == Some(0) && out.trim() == "42";
                (ok, format!("python3 app.py -> {:?} (exit {:?})", out.trim(), code))
            },
        },
        Scenario {
            name: "fixtest",
            blurb: "read a failing test, fix the bug, make it pass",
            setup: |dir| {
                // Buggy implementation: subtracts instead of adds.
                std::fs::write(dir.join("calc.py"), "def add(a, b):\n    return a - b\n")?;
                std::fs::write(
                    dir.join("test_calc.py"),
                    "from calc import add\n\
                     assert add(2, 3) == 5, 'add(2,3) should be 5'\n\
                     assert add(10, 5) == 15, 'add(10,5) should be 15'\n\
                     print('tests passed')\n",
                )?;
                Ok(())
            },
            task: "Run `python3 test_calc.py`. The assertions fail because of a bug in calc.py. \
                   Fix calc.py so the tests pass. Do not modify the test file.",
            verify: |dir| {
                let (code, out, _) = run(dir, "python3", &["test_calc.py"]);
                let ok = code == Some(0) && out.contains("tests passed");
                (ok, format!("tests exit {:?}, stdout {:?}", code, out.trim()))
            },
        },
        Scenario {
            name: "debug",
            blurb: "run, read the traceback, fix the runtime error",
            setup: |dir| {
                // NameError: `greeting` is never defined.
                std::fs::write(dir.join("crash.py"), "def main():\n    print(greeting)\n\nmain()\n")
            },
            task: "Run `python3 crash.py`. It crashes with an error. Diagnose the cause and fix \
                   crash.py so that running it prints exactly: ok",
            verify: |dir| {
                let (code, out, _) = run(dir, "python3", &["crash.py"]);
                let ok = code == Some(0) && out.trim() == "ok";
                (ok, format!("python3 crash.py -> {:?} (exit {:?})", out.trim(), code))
            },
        },
        Scenario {
            name: "refactor",
            blurb: "rename a symbol across files, keep it working",
            setup: |dir| {
                std::fs::write(
                    dir.join("greet.py"),
                    "def greet_user(name):\n    return f'hi {name}'\n",
                )?;
                std::fs::write(
                    dir.join("main.py"),
                    "from greet import greet_user\nprint(greet_user('sam'))\n",
                )?;
                Ok(())
            },
            task: "Rename the function `greet_user` to `welcome_user` everywhere — the definition \
                   in greet.py and every call site (including in main.py). Then run \
                   `python3 main.py` to confirm it still works.",
            verify: |dir| {
                let still_old = grep_py(dir, "greet_user");
                let (code, out, _) = run(dir, "python3", &["main.py"]);
                let ok = !still_old && code == Some(0) && out.contains("hi sam");
                (
                    ok,
                    format!(
                        "old name present: {}; python3 main.py -> {:?} (exit {:?})",
                        still_old,
                        out.trim(),
                        code
                    ),
                )
            },
        },
    ]
}

/// Look up a scenario by name.
pub fn find(name: &str) -> Option<Scenario> {
    all().into_iter().find(|s| s.name == name)
}

/// Run a command in `dir`, returning (exit_code, stdout, stderr).
fn run(dir: &Path, cmd: &str, args: &[&str]) -> (Option<i32>, String, String) {
    match Command::new(cmd).args(args).current_dir(dir).output() {
        Ok(o) => (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).to_string(),
            String::from_utf8_lossy(&o.stderr).to_string(),
        ),
        Err(e) => (None, String::new(), e.to_string()),
    }
}

/// Whether `needle` still appears in any `.py` file under `dir`.
fn grep_py(dir: &Path, needle: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("py")
            && let Ok(content) = std::fs::read_to_string(&path)
            && content.contains(needle)
        {
            return true;
        }
    }
    false
}
