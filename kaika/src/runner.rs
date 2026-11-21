//! Every external tool runs through a `Runner`, so tests can substitute stubs.

use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Default)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
}

pub fn argv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

pub trait Runner {
    /// Run to completion, capturing output. Err only when the program could not start.
    fn run(&self, argv: &[String]) -> Result<Out>;
    /// Like `run`, feeding `input` on stdin.
    fn run_stdin(&self, argv: &[String], input: &str) -> Result<Out>;
    /// Run with inherited stdio, so long installs show their own progress. Returns the exit code.
    fn run_live(&self, argv: &[String]) -> Result<i32>;
    /// Is this program on PATH?
    fn exists(&self, prog: &str) -> bool;
}

pub struct RealRunner;

fn split(argv: &[String]) -> Result<(&String, &[String])> {
    argv.split_first().ok_or_else(|| anyhow!("empty command"))
}

fn collect(o: std::process::Output) -> Out {
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

impl Runner for RealRunner {
    fn run(&self, argv: &[String]) -> Result<Out> {
        let (p, a) = split(argv)?;
        let o = Command::new(p).args(a).stdin(Stdio::null()).output().map_err(|e| anyhow!("{p}: {e}"))?;
        Ok(collect(o))
    }

    fn run_stdin(&self, argv: &[String], input: &str) -> Result<Out> {
        let (p, a) = split(argv)?;
        let mut child = Command::new(p)
            .args(a)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("{p}: {e}"))?;
        if let Some(mut si) = child.stdin.take() {
            // A child that exits without reading stdin closes the pipe; that is not our error.
            let _ = si.write_all(input.as_bytes());
        }
        Ok(collect(child.wait_with_output()?))
    }

    fn run_live(&self, argv: &[String]) -> Result<i32> {
        let (p, a) = split(argv)?;
        let s = Command::new(p).args(a).status().map_err(|e| anyhow!("{p}: {e}"))?;
        Ok(s.code().unwrap_or(-1))
    }

    fn exists(&self, prog: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;
        let Some(path) = std::env::var_os("PATH") else { return false };
        std::env::split_paths(&path).any(|d| {
            std::fs::metadata(d.join(prog)).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        })
    }
}

/// A scripted runner for unit tests: responses keyed by the space-joined argv.
#[derive(Default)]
pub struct Fake {
    pub responses: HashMap<String, Out>,
    /// Successive answers to the same command (a decision asks twice, in two candidate orders).
    pub sequences: RefCell<HashMap<String, VecDeque<Out>>>,
    pub tools: Vec<String>,
    pub log: RefCell<Vec<String>>,
}

impl Fake {
    pub fn with(mut self, cmd: &str, stdout: &str) -> Self {
        self.responses.insert(cmd.into(), Out { code: 0, stdout: stdout.into(), stderr: String::new() });
        self
    }
    pub fn then(self, cmd: &str, outputs: &[&str]) -> Self {
        let q = outputs.iter().map(|o| Out { code: 0, stdout: o.to_string(), stderr: String::new() }).collect();
        self.sequences.borrow_mut().insert(cmd.into(), q);
        self
    }
    pub fn failing(mut self, cmd: &str) -> Self {
        self.responses.insert(cmd.into(), Out { code: 1, ..Default::default() });
        self
    }
    pub fn tool(mut self, t: &str) -> Self {
        self.tools.push(t.into());
        self
    }
}

impl Runner for Fake {
    fn run(&self, argv: &[String]) -> Result<Out> {
        let key = argv.join(" ");
        self.log.borrow_mut().push(key.clone());
        if let Some(next) = self.sequences.borrow_mut().get_mut(&key).and_then(|q| q.pop_front()) {
            return Ok(next);
        }
        Ok(self.responses.get(&key).cloned().unwrap_or(Out { code: 1, ..Default::default() }))
    }
    fn run_stdin(&self, argv: &[String], _input: &str) -> Result<Out> {
        self.run(argv)
    }
    fn run_live(&self, argv: &[String]) -> Result<i32> {
        Ok(self.run(argv)?.code)
    }
    fn exists(&self, prog: &str) -> bool {
        self.tools.iter().any(|t| t == prog)
    }
}
