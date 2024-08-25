use crate::{
    error::Error,
    io::{FileSystemWriter, Stdio},
    paths, Result,
};

use std::{
    collections::HashSet,
    io::{self, BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command},
};

use camino::{Utf8Path, Utf8PathBuf};

#[derive(Debug)]
struct BeamCompilerInner {
    process: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug, Default)]
pub struct BeamCompiler {
    inner: Option<BeamCompilerInner>,
}

impl BeamCompiler {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn compile<IO: FileSystemWriter>(
        &mut self,
        io: &IO,
        out: &Utf8Path,
        lib: &Utf8Path,
        modules: &HashSet<Utf8PathBuf>,
        stdio: Stdio,
    ) -> Result<(), Error> {
        let inner = match self.inner {
            Some(ref mut inner) => {
                if let Ok(None) = inner.process.try_wait() {
                    inner
                } else {
                    self.inner.insert(self.spawn(io, out)?)
                }
            }

            None => self.inner.insert(self.spawn(io, out)?),
        };

        let mut args = vec![
            "--lib".into(),
            lib.to_string(),
            "--out".into(),
            out.join("ebin").to_string(),
        ];

        for module in modules {
            let path = out.join(paths::ARTEFACT_DIRECTORY_NAME).join(module);
            args.push(path.to_string());
        }

        tracing::debug!(args=?args.join(" "), "call_beam_compiler");

        writeln!(inner.stdin, "{}", args.join("\x1f")).map_err(|e| Error::ShellCommand {
            program: "escript".into(),
            err: Some(e.kind()),
        })?;

        let mut buf = String::new();
        while let Ok(_) = inner.stdout.read_line(&mut buf) {
            match buf.trim() {
                "ok" => return Ok(()),
                "err" => {
                    return Err(Error::ShellCommand {
                        program: "escript".into(),
                        err: None,
                    })
                }
                _ => match stdio {
                    Stdio::Inherit => print!("{}", buf),
                    Stdio::Null => {}
                },
            }

            buf.clear()
        }

        // if we get here, stdout got closed before we got an "ok" or "err".
        Err(Error::ShellCommand {
            program: "escript".into(),
            err: None,
        })
    }

    fn spawn<IO: FileSystemWriter>(
        &self,
        io: &IO,
        out: &Utf8Path,
    ) -> Result<BeamCompilerInner, Error> {
        let escript_path = out
            .join(paths::ARTEFACT_DIRECTORY_NAME)
            .join("gleam@@compile.erl");

        let escript_source = std::include_str!("../../templates/gleam@@compile.erl");
        io.write(&escript_path, escript_source)?;

        tracing::trace!(escript_path=?escript_path, "spawn_beam_compiler");

        let mut process = std::process::Command::new("escript")
            .args([escript_path])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => Error::ShellProgramNotFound {
                    program: "escript".into(),
                },
                other => Error::ShellCommand {
                    program: "escript".into(),
                    err: Some(other),
                },
            })?;

        let stdin = process.stdin.take().expect("could not get child stdin");
        let stdout = process.stdout.take().expect("could not get child stdout");

        Ok(BeamCompilerInner {
            process,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }
}

impl Drop for BeamCompiler {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            let _ = inner.process.kill();
            let _ = inner.process.wait();
        }
    }
}