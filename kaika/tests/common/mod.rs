//! A sandbox for hermetic tests: temp HOME, stub executables first on PATH, no network, no GUI.

use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct Sandbox {
    pub root: TempDir,
    pub stubs: PathBuf,
    pub resp: PathBuf,
    pub log: PathBuf,
    pub home: PathBuf,
    pub apps: PathBuf,
}

#[derive(Clone, Copy)]
pub enum Match {
    /// Key on every argument.
    Exact,
    /// Key on every argument except the last (a path that varies per run).
    IgnoreLast,
    /// Key on the tool name alone.
    Any,
}

pub fn key(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

impl Sandbox {
    pub fn new() -> Sandbox {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().to_path_buf();
        let s = Sandbox { stubs: p.join("stubs"), resp: p.join("resp"), log: p.join("calls.log"), home: p.join("home"), apps: p.join("Applications"), root };
        for d in [&s.stubs, &s.resp, &s.home, &s.apps] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(&s.log, "").unwrap();
        s
    }

    /// Install a stub for `tool`. Unscripted calls print nothing and exit `default_code`.
    pub fn stub(&self, tool: &str, default_code: i32, mode: Match) -> &Sandbox {
        let ignore = matches!(mode, Match::IgnoreLast) as i32;
        let any = matches!(mode, Match::Any) as i32;
        let passthrough = if tool == "curl" {
            "for a in \"$@\"; do case \"$a\" in file://*) exec /usr/bin/curl \"$@\";; esac; done\n"
        } else {
            ""
        };
        let script = format!(
            r#"#!/bin/sh
{passthrough}echo "{tool} $*" >> "$KAIKA_STUB_LOG"
text="{tool}"
if [ {any} -ne 1 ]; then
  n=$#; i=0
  for a in "$@"; do
    i=$((i+1))
    if [ {ignore} -eq 1 ] && [ $i -eq $n ]; then break; fi
    text="$text $a"
  done
fi
text=$(printf '%s' "$text" | sed -E 's/127\.0\.0\.1:[0-9]+/127.0.0.1:PORT/g')
key=$(printf '%s' "$text" | tr -c 'A-Za-z0-9' '_')
f="$KAIKA_STUB_DIR/$key"
n=0; [ -f "$f.n" ] && n=$(cat "$f.n"); n=$((n+1)); echo $n > "$f.n"
out="$f.out"; [ -f "$f.out.$n" ] && out="$f.out.$n"
[ -f "$f.err" ] && cat "$f.err" >&2
[ -f "$out" ] && cat "$out"
if [ -f "$f.code" ]; then exit "$(cat "$f.code")"; fi
if [ -f "$out" ]; then exit 0; fi
exit {default_code}
"#
        );
        let path = self.stubs.join(tool);
        fs::write(&path, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        self
    }

    /// Install a hand-written stub script (for tools whose behaviour depends on their arguments).
    pub fn custom_stub(&self, tool: &str, body: &str) -> &Sandbox {
        let path = self.stubs.join(tool);
        fs::write(&path, format!("#!/bin/sh\necho \"{tool} $*\" >> \"$KAIKA_STUB_LOG\"\n{body}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        self
    }

    /// Stand-in for `hdiutil`: `attach` fills the mountpoint with `Fixture.app` (copied from `app_src`)
    /// instead of mounting anything, so no disk image is ever mounted on the host.
    pub fn fake_hdiutil(&self, app_src: &Path, attach_ok: bool) -> &Sandbox {
        let body = format!(
            r#"case "$1" in
attach)
  {fail}
  while [ $# -gt 0 ]; do [ "$1" = "-mountpoint" ] && mnt="$2"; shift; done
  /usr/bin/ditto "{src}" "$mnt/Fixture.app" ;;
detach) exit 0 ;;
esac"#,
            fail = if attach_ok { "" } else { "echo 'image not recognized' >&2; exit 1" },
            src = app_src.display()
        );
        self.custom_stub("hdiutil", &body)
    }

    /// The unpacked `Fixture.app` used by `fixture_zip`.
    pub fn fixture_app(&self) -> PathBuf {
        self.fixture_zip("unused.zip");
        self.root.path().join("src/Fixture.app")
    }

    /// Script what `text` (tool plus arguments, as logged) prints on stdout, optionally stderr and an exit code.
    pub fn reply(&self, text: &str, stdout: &str) -> &Sandbox {
        fs::write(self.resp.join(format!("{}.out", key(text))), stdout).unwrap();
        self
    }
    /// Successive stdout answers to the same command: the first call gets `outputs[0]`, the second `outputs[1]`...
    pub fn reply_seq(&self, text: &str, outputs: &[&str]) -> &Sandbox {
        for (i, o) in outputs.iter().enumerate() {
            fs::write(self.resp.join(format!("{}.out.{}", key(text), i + 1)), o).unwrap();
        }
        self
    }
    pub fn reply_err(&self, text: &str, stderr: &str) -> &Sandbox {
        fs::write(self.resp.join(format!("{}.err", key(text))), stderr).unwrap();
        self
    }
    pub fn reply_code(&self, text: &str, code: i32) -> &Sandbox {
        fs::write(self.resp.join(format!("{}.code", key(text))), code.to_string()).unwrap();
        self
    }

    pub fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_kaika"));
        c.env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.stubs.display()))
            .env("HOME", &self.home)
            .env("KAIKA_APPS_DIR", &self.apps)
            .env("KAIKA_MODELS", "off")
            .env("KAIKA_TEST_MODE", "1")
            .env("KAIKA_STUB_DIR", &self.resp)
            .env("KAIKA_STUB_LOG", &self.log);
        c
    }

    pub fn calls(&self) -> String {
        fs::read_to_string(&self.log).unwrap()
    }

    /// kaika's dedicated home: config, recipes, manifest, bin, packages and cache all live here.
    pub fn kaika_home(&self) -> PathBuf {
        self.home.join(".kaika")
    }
    pub fn data(&self) -> PathBuf {
        self.kaika_home()
    }
    pub fn config_dir(&self) -> PathBuf {
        self.kaika_home()
    }
    pub fn manifest(&self) -> String {
        fs::read_to_string(self.data().join("manifest.toml")).unwrap_or_default()
    }

    /// Standard stubs: package managers answer nothing; verification tools pass.
    pub fn standard(&self) -> &Sandbox {
        for t in ["brew", "npm", "uv", "mas"] {
            self.stub(t, 0, Match::Exact);
        }
        self.stub("curl", 22, Match::Exact);
        self.stub("codesign", 0, Match::IgnoreLast);
        self.stub("spctl", 0, Match::IgnoreLast);
        self.stub("defaults", 0, Match::Any);
        self.stub("hdiutil", 0, Match::Exact);
        self.stub("sudo", 0, Match::Exact);
        self.good_signature("ABCDE12345");
        self.reply(Sandbox::PROBE, "");
        self
    }

    pub fn good_signature(&self, team: &str) -> &Sandbox {
        self.reply_err("codesign -dv --verbose=4", &format!("Identifier=com.fixture.app\nTeamIdentifier={team}\n"));
        self.reply_err("spctl -a -vv", "accepted\nsource=Notarized Developer ID\n");
        self
    }

    /// A zip containing `Fixture.app`, built with the system `ditto`. Returns its path.
    pub fn fixture_zip(&self, name: &str) -> PathBuf {
        let src = self.root.path().join("src");
        let app = src.join("Fixture.app/Contents/MacOS");
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join("fixture"), "#!/bin/sh\n").unwrap();
        fs::write(src.join("Fixture.app/Contents/Info.plist"), "<plist/>").unwrap();
        let zip = self.root.path().join(name);
        let st = std::process::Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(src.join("Fixture.app"))
            .arg(&zip)
            .status()
            .unwrap();
        assert!(st.success());
        zip
    }

    /// What llama-server answers on /v1/systemone: the probabilities of each option.
    pub fn systemone(probs: &[(&str, f64)]) -> String {
        let p: Vec<String> = probs.iter().map(|(k, v)| format!("{k:?}: {v}")).collect();
        format!(r#"{{"model":"h2oai/h2o-lightning-4b","answers":{{"q":{{"type":"choice","choice":"{}","probabilities":{{{}}}}}}}}}"#, probs[0].0, p.join(","))
    }

    /// The (port-normalised) curl invocations kaika makes against its own model server.
    pub const ASK: &'static str = "curl -s --max-time 60 -H Content-Type: application/json -d @- http://127.0.0.1:PORT/v1/systemone";
    pub const HEALTH: &'static str = "curl -sf --max-time 1 http://127.0.0.1:PORT/health";
    /// What the connectivity probe runs (answering it means "online").
    pub const PROBE: &'static str = "curl -s -o /dev/null -I --connect-timeout 2 --max-time 3 https://www.wikidata.org/";

    /// A stand-in `llama-server`: `--version` prints a build number; started as a server it records its pid and
    /// its arguments, then sleeps until it is killed.
    pub fn stub_llama(&self, build: u32) -> &Sandbox {
        let pid = self.resp.join("llama.pid");
        self.custom_stub(
            "llama-server",
            &format!(
                r#"case "$1" in --version) echo "version: 1.0.0 (build {build}, commit stub)"; exit 0;; esac
echo $$ > "{}"
exec sleep 600"#,
                pid.display()
            ),
        )
    }

    pub fn llama_pid(&self) -> Option<u32> {
        fs::read_to_string(self.resp.join("llama.pid")).ok().and_then(|t| t.trim().parse().ok())
    }

    pub fn pid_alive(pid: u32) -> bool {
        std::process::Command::new("kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    }

    /// Wait up to `secs` for a process to be gone.
    pub fn gone_within(pid: u32, secs: u64) -> bool {
        for _ in 0..secs * 20 {
            if !Sandbox::pid_alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }

    /// A small file standing in for the model, at the place kaika looks for it.
    pub fn put_model(&self) -> &Sandbox {
        let d = self.kaika_home().join("models/h2o-lightning");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("h2o-lightning-4b-Q4_K_M.gguf"), "not a real model").unwrap();
        self
    }

    /// Everything the model needs: llama-server, the model file, and a server that answers `health`.
    pub fn model_ready(&self) -> &Sandbox {
        self.stub_llama(12359).put_model().reply(Sandbox::HEALTH, r#"{"status":"ok"}"#)
    }

    pub fn recipe(&self, file: &str, body: &str) {
        let d = self.config_dir().join("recipes");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(file), body).unwrap();
    }

    pub fn url(path: &Path) -> String {
        format!("file://{}", path.display())
    }
}
