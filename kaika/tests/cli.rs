mod common;
use common::{Match, Sandbox};
use predicates::prelude::*;

fn sandbox() -> Sandbox {
    let s = Sandbox::new();
    s.standard();
    s
}

// ---------- list, search, info ----------

#[test]
fn list_merges_every_source_and_hides_apps_a_cask_owns() {
    let s = sandbox();
    s.reply("brew leaves", "jq\n")
        .reply("brew list --formula --versions", "jq 1.7\n")
        .reply("brew list --cask --versions", "zen 1.0\n");
    std::fs::create_dir(s.apps.join("Zen.app")).unwrap();
    std::fs::create_dir(s.apps.join("Safari.app")).unwrap();
    let out = s.cmd().args(["list", "--json"]).assert().success().get_output().stdout.clone();
    let rows: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let names: Vec<String> = rows.as_array().unwrap().iter().map(|r| format!("{}:{}", r["source"].as_str().unwrap(), r["name"].as_str().unwrap())).collect();
    assert!(names.contains(&"brew:jq".into()) && names.contains(&"cask:zen".into()) && names.contains(&"app:Safari".into()), "{names:?}");
    assert!(!names.contains(&"app:Zen".into()));
}

#[test]
fn search_shows_backend_hits_and_recipes() {
    let s = sandbox();
    s.reply("brew search --formula rect", "rectangle-formula\n");
    s.cmd().args(["search", "rect"]).assert().success().stdout(predicate::str::contains("brew:rectangle-formula").and(predicate::str::contains("direct:rectangle")));
}

#[test]
fn search_with_no_results_fails() {
    sandbox().cmd().args(["search", "zzzznothing"]).assert().code(1);
}

#[test]
fn info_for_an_unknown_name_fails_cleanly() {
    sandbox().cmd().args(["info", "zzzznothing"]).assert().code(1).stderr(predicate::str::contains("nothing known"));
}

// ---------- install: plan, confirm, force ----------

fn with_ripgrep(s: &Sandbox) {
    s.reply("brew search --formula ripgrep", "ripgrep\nripgrep-all\n").reply("brew list --formula --versions", "ripgrep 14.0\n").reply("brew leaves", "ripgrep\n");
}

#[test]
fn dry_run_prints_the_plan_and_changes_nothing() {
    let s = sandbox();
    with_ripgrep(&s);
    s.cmd().args(["install", "ripgrep", "-n"]).assert().success().stdout(predicate::str::contains("brew install ripgrep"));
    assert!(!s.calls().contains("brew install"), "{}", s.calls());
    assert!(s.manifest().is_empty());
}

#[test]
fn no_terminal_and_no_yes_exits_4_without_changing_anything() {
    let s = sandbox();
    with_ripgrep(&s);
    s.cmd().args(["install", "ripgrep"]).assert().code(4).stdout(predicate::str::contains("brew install ripgrep")).stderr(predicate::str::contains("-y"));
    assert!(!s.calls().contains("brew install"));
}

#[test]
fn force_does_not_skip_the_prompt() {
    let s = sandbox();
    with_ripgrep(&s);
    s.cmd().args(["install", "ripgrep", "-f"]).assert().code(4);
    assert!(!s.calls().contains("brew install"));
}

#[test]
fn yes_runs_the_backend_and_records_the_manifest() {
    let s = sandbox();
    with_ripgrep(&s);
    s.cmd().args(["install", "ripgrep", "-y"]).assert().success();
    assert!(s.calls().contains("brew install ripgrep"));
    let m = s.manifest();
    assert!(m.contains("name = \"ripgrep\"") && m.contains("source = \"brew\"") && m.contains("version = \"14.0\""), "{m}");
}

#[test]
fn an_interactive_no_cancels_and_a_yes_proceeds() {
    let s = sandbox();
    with_ripgrep(&s);
    s.cmd().env("KAIKA_ASSUME_TTY", "1").write_stdin("n\n").args(["install", "ripgrep"]).assert().code(1).stderr(predicate::str::contains("cancelled"));
    assert!(!s.calls().contains("brew install"));
    s.cmd().env("KAIKA_ASSUME_TTY", "1").write_stdin("y\n").args(["install", "ripgrep"]).assert().success();
    assert!(s.calls().contains("brew install ripgrep"));
}

#[test]
fn an_unknown_name_fails_and_changes_nothing() {
    let s = sandbox();
    s.cmd().args(["install", "zzzznothing", "-y"]).assert().code(1).stderr(predicate::str::contains("no package named"));
    assert!(!s.calls().contains("install"));
}

#[test]
fn a_failing_backend_surfaces_and_is_not_recorded() {
    let s = sandbox();
    with_ripgrep(&s);
    s.reply_code("brew install ripgrep", 1);
    s.cmd().args(["install", "ripgrep", "-y"]).assert().code(1).stderr(predicate::str::contains("failed"));
    assert!(s.manifest().is_empty());
}

#[test]
fn a_bad_source_prefix_is_a_usage_error() {
    sandbox().cmd().args(["install", "pip:requests", "-n"]).assert().code(2);
}

#[test]
fn ambiguous_names_follow_the_preference_order_and_say_so() {
    let s = sandbox();
    s.reply("brew search --formula docker", "docker\n").reply("brew search --cask docker", "docker\n");
    s.cmd().args(["install", "docker", "-n"]).assert().success().stdout(predicate::str::contains("brew install docker").and(predicate::str::contains("ambiguous")).and(predicate::str::contains("cask:docker")));
}

#[test]
fn source_prefix_forces_the_backend() {
    let s = sandbox();
    s.reply("brew search --formula docker", "docker\n").reply("brew search --cask docker", "docker\n");
    s.cmd().args(["install", "cask:docker", "-n"]).assert().success().stdout(predicate::str::contains("brew install --cask docker"));
}

#[test]
fn config_order_changes_the_winner() {
    let s = sandbox();
    s.reply("brew search --formula docker", "docker\n").reply("brew search --cask docker", "docker\n");
    s.cmd().args(["config", "set", "order", "cask,brew"]).assert().success();
    s.cmd().args(["install", "docker", "-n"]).assert().success().stdout(predicate::str::contains("brew install --cask docker"));
}

// ---------- uninstall, upgrade ----------

#[test]
fn uninstall_zap_uses_the_casks_own_flag_and_forgets_the_package() {
    let s = sandbox();
    s.reply("brew list --cask --versions", "zen 1.0\n");
    s.cmd().args(["uninstall", "zen", "--zap", "-y"]).assert().success();
    assert!(s.calls().contains("brew uninstall --cask --zap zen"), "{}", s.calls());
}

#[test]
fn remove_is_an_alias_for_uninstall() {
    let s = sandbox();
    s.reply("brew list --cask --versions", "zen 1.0\n");
    s.cmd().args(["remove", "zen", "-n"]).assert().success().stdout(predicate::str::contains("uninstall --cask zen"));
}

#[test]
fn uninstalling_something_not_installed_fails() {
    sandbox().cmd().args(["uninstall", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("not installed"));
}

#[test]
fn a_hand_managed_app_is_listed_but_never_touched() {
    let s = sandbox();
    std::fs::create_dir(s.apps.join("Safari.app")).unwrap();
    s.cmd().args(["uninstall", "Safari", "-y"]).assert().code(1).stderr(predicate::str::contains("managed by hand"));
}

#[test]
fn a_name_in_two_sources_needs_source_prefix() {
    let s = sandbox();
    s.reply("brew leaves", "dup\n").reply("brew list --formula --versions", "dup 1\n").reply("brew list --cask --versions", "dup 2\n");
    s.cmd().args(["uninstall", "dup", "-y"]).assert().code(2).stderr(predicate::str::contains("source:name"));
}

#[test]
fn upgrade_needs_names_or_all() {
    sandbox().cmd().args(["upgrade"]).assert().code(2);
}

#[test]
fn upgrade_all_asks_every_available_backend() {
    let s = sandbox();
    s.cmd().args(["upgrade", "--all", "-y"]).assert().success();
    let c = s.calls();
    for want in ["brew upgrade --formula", "brew upgrade --cask", "npm update -g", "uv tool upgrade --all", "mas upgrade"] {
        assert!(c.contains(want), "missing {want} in {c}");
    }
}

#[test]
fn a_backend_that_is_not_installed_is_skipped() {
    let s = Sandbox::new();
    s.stub("brew", 0, Match::Exact);
    s.cmd().args(["upgrade", "--all", "-y"]).assert().success();
    assert!(!s.calls().contains("npm") && s.calls().contains("brew upgrade"));
}

// ---------- the direct backend: verify, install, remove ----------

fn fixture_recipe(s: &Sandbox, extra: &str) -> std::path::PathBuf {
    let zip = s.fixture_zip("Fixture-1.0.zip");
    s.recipe("fixture.toml", &format!("name = \"fixture\"\nurl = \"{}\"\napp = \"Fixture.app\"\n{extra}", Sandbox::url(&zip)));
    zip
}

#[test]
fn a_verified_download_installs_and_records_the_team_id() {
    let s = sandbox();
    fixture_recipe(&s, "");
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().success().stdout(predicate::str::contains("ABCDE12345"));
    assert!(s.apps.join("Fixture.app/Contents/Info.plist").exists());
    let m = s.manifest();
    assert!(m.contains("source = \"direct\"") && m.contains("team_id = \"ABCDE12345\""), "{m}");
}

#[test]
fn a_wrong_team_id_stops_the_install_with_exit_3() {
    let s = sandbox();
    fixture_recipe(&s, "team_id = \"ZZZZZZZZZZ\"\n");
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(3).stderr(predicate::str::contains("does not match").and(predicate::str::contains("nothing was installed")));
    assert!(!s.apps.join("Fixture.app").exists());
    assert!(s.manifest().is_empty());
}

#[test]
fn force_overrides_a_failed_check_and_says_so_loudly() {
    let s = sandbox();
    fixture_recipe(&s, "team_id = \"ZZZZZZZZZZ\"\n");
    s.cmd().args(["install", "direct:fixture", "-y", "-f"]).assert().success().stderr(predicate::str::contains("WARNING").and(predicate::str::contains("does not match")));
    assert!(s.apps.join("Fixture.app").exists());
}

#[test]
fn the_plan_announces_that_force_overrides_verification() {
    let s = sandbox();
    fixture_recipe(&s, "");
    s.cmd().args(["install", "direct:fixture", "-n", "-f"]).assert().success().stdout(predicate::str::contains("-f: verification failures will be overridden"));
}

#[test]
fn a_checksum_mismatch_is_exit_3() {
    let s = sandbox();
    fixture_recipe(&s, "sha256 = \"00\"\n");
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(3).stderr(predicate::str::contains("checksum mismatch"));
    assert!(!s.apps.join("Fixture.app").exists());
}

#[test]
fn a_correct_checksum_passes() {
    let s = sandbox();
    let zip = fixture_recipe(&s, "");
    let sum = kaika::verify::sha256_file(&zip).unwrap();
    s.recipe("fixture.toml", &format!("name = \"fixture\"\nurl = \"{}\"\napp = \"Fixture.app\"\nsha256 = \"{sum}\"\n", Sandbox::url(&zip)));
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().success();
}

#[test]
fn an_unnotarized_app_is_refused() {
    let s = sandbox();
    fixture_recipe(&s, "");
    s.reply_err("spctl -a -vv", "rejected\n").reply_code("spctl -a -vv", 3);
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(3).stderr(predicate::str::contains("not notarized"));
}

#[test]
fn an_unsigned_app_is_refused() {
    let s = sandbox();
    fixture_recipe(&s, "");
    s.reply_code("codesign --verify --deep --strict", 1);
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(3).stderr(predicate::str::contains("signature"));
}

#[test]
fn plain_http_is_never_downloaded() {
    let s = sandbox();
    s.recipe("fixture.toml", "name = \"fixture\"\nurl = \"http://example.com/Fixture.zip\"\n");
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(1).stderr(predicate::str::contains("non-HTTPS"));
    assert!(!s.calls().contains("curl -fSL"), "no download may start: {}", s.calls());
}

#[test]
fn an_existing_app_is_not_overwritten_without_force() {
    let s = sandbox();
    fixture_recipe(&s, "");
    std::fs::create_dir_all(s.apps.join("Fixture.app")).unwrap();
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(1).stderr(predicate::str::contains("already exists"));
    s.cmd().args(["install", "direct:fixture", "-y", "-f"]).assert().success();
}

#[test]
fn uninstalling_a_direct_app_trashes_it_and_zap_removes_support_files() {
    let s = sandbox();
    fixture_recipe(&s, "");
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().success();
    let support = s.home.join("Library/Application Support/Fixture");
    let cache = s.home.join("Library/Caches/com.fixture.app");
    std::fs::create_dir_all(&support).unwrap();
    std::fs::create_dir_all(&cache).unwrap();
    s.reply("defaults", "com.fixture.app\n");
    s.cmd().args(["uninstall", "fixture", "--zap", "-y"]).assert().success();
    assert!(!s.apps.join("Fixture.app").exists());
    assert!(s.home.join(".Trash/Fixture.app").exists());
    assert!(!support.exists() && !cache.exists());
    assert!(!s.manifest().contains("fixture"));
}

#[test]
fn uninstall_without_zap_keeps_support_files() {
    let s = sandbox();
    fixture_recipe(&s, "");
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().success();
    let support = s.home.join("Library/Application Support/Fixture");
    std::fs::create_dir_all(&support).unwrap();
    s.cmd().args(["uninstall", "fixture", "-y"]).assert().success();
    assert!(support.exists());
}

// ---------- the other download formats: dmg, tar.gz, pkg ----------

#[test]
fn a_dmg_is_opened_verified_installed_and_always_detached() {
    let s = sandbox();
    let app = s.fixture_app();
    s.fake_hdiutil(&app, true);
    let dmg = s.root.path().join("Fixture-1.0.dmg");
    std::fs::write(&dmg, "not a real image").unwrap();
    s.recipe("fixture.toml", &format!("name = \"fixture\"\nurl = \"{}\"\napp = \"Fixture.app\"\n", Sandbox::url(&dmg)));
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().success();
    assert!(s.apps.join("Fixture.app/Contents/Info.plist").exists());
    let calls = s.calls();
    assert!(calls.contains("hdiutil attach -nobrowse -noverify -readonly -quiet") && calls.contains("hdiutil detach"), "{calls}");
    assert!(!s.kaika_home().join("cache/mnt").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false), "mount dir left behind");
}

#[test]
fn a_dmg_that_fails_verification_is_still_detached_and_nothing_is_installed() {
    let s = sandbox();
    let app = s.fixture_app();
    s.fake_hdiutil(&app, true);
    s.reply_code("codesign --verify --deep --strict", 1);
    let dmg = s.root.path().join("Fixture-1.0.dmg");
    std::fs::write(&dmg, "x").unwrap();
    s.recipe("fixture.toml", &format!("name = \"fixture\"\nurl = \"{}\"\n", Sandbox::url(&dmg)));
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(3);
    assert!(s.calls().contains("hdiutil detach"));
    assert!(!s.apps.join("Fixture.app").exists());
}

#[test]
fn a_dmg_that_will_not_open_is_a_clear_failure() {
    let s = sandbox();
    let app = s.fixture_app();
    s.fake_hdiutil(&app, false);
    let dmg = s.root.path().join("Fixture-1.0.dmg");
    std::fs::write(&dmg, "x").unwrap();
    s.recipe("fixture.toml", &format!("name = \"fixture\"\nurl = \"{}\"\n", Sandbox::url(&dmg)));
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(1).stderr(predicate::str::contains("could not open the disk image"));
}

fn tarball(s: &Sandbox) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let src = s.root.path().join("tarsrc/tool-1.0");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("tool"), "#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(src.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(src.join("README"), "docs").unwrap();
    let tgz = s.root.path().join("tool-1.0.tar.gz");
    assert!(std::process::Command::new("/usr/bin/tar").args(["-czf"]).arg(&tgz).arg("-C").arg(s.root.path().join("tarsrc")).arg("tool-1.0").status().unwrap().success());
    tgz
}

#[test]
fn a_command_line_package_is_unpacked_whole_and_linked_into_bin() {
    let s = sandbox();
    let tgz = tarball(&s);
    let sum = kaika::verify::sha256_file(&tgz).unwrap();
    s.recipe("tool.toml", &format!("name = \"tool\"\nurl = \"{}\"\nsha256 = \"{sum}\"\n", Sandbox::url(&tgz)));
    s.cmd().args(["install", "direct:tool", "-y"]).assert().success();
    let pkg = s.kaika_home().join("packages/tool/latest");
    assert!(pkg.join("tool").exists() && pkg.join("README").exists(), "the whole package is kept, not just the binary");
    let link = s.kaika_home().join("bin/tool");
    assert_eq!(std::fs::read_link(&link).unwrap(), pkg.join("tool"));
    assert!(s.manifest().contains("name = \"tool\"") && s.manifest().contains("packages/tool/latest"));
}

#[test]
fn a_tarball_with_no_checksum_is_unverifiable_and_refused_without_force() {
    let s = sandbox();
    let tgz = tarball(&s);
    s.recipe("tool.toml", &format!("name = \"tool\"\nurl = \"{}\"\n", Sandbox::url(&tgz)));
    s.cmd().args(["install", "direct:tool", "-y"]).assert().code(3).stderr(predicate::str::contains("no published checksum"));
    assert!(!s.kaika_home().join("bin/tool").exists() && !s.kaika_home().join("packages").exists());
    s.cmd().args(["install", "direct:tool", "-y", "-f"]).assert().success().stderr(predicate::str::contains("WARNING"));
    assert!(s.kaika_home().join("bin/tool").exists());
}

#[test]
fn uninstalling_a_command_line_package_removes_its_links_and_its_folder() {
    let s = sandbox();
    let tgz = tarball(&s);
    let sum = kaika::verify::sha256_file(&tgz).unwrap();
    s.recipe("tool.toml", &format!("name = \"tool\"\nurl = \"{}\"\nsha256 = \"{sum}\"\n", Sandbox::url(&tgz)));
    s.cmd().args(["install", "direct:tool", "-y"]).assert().success();
    s.cmd().args(["uninstall", "tool", "-y"]).assert().success();
    assert!(std::fs::symlink_metadata(s.kaika_home().join("bin/tool")).is_err());
    assert!(!s.kaika_home().join("packages/tool").exists());
    assert!(!s.manifest().contains("name = \"tool\""));
}

#[test]
fn a_file_in_bin_that_kaika_did_not_put_there_is_not_overwritten_without_force() {
    let s = sandbox();
    let tgz = tarball(&s);
    let sum = kaika::verify::sha256_file(&tgz).unwrap();
    s.recipe("tool.toml", &format!("name = \"tool\"\nurl = \"{}\"\nsha256 = \"{sum}\"\n", Sandbox::url(&tgz)));
    std::fs::create_dir_all(s.kaika_home().join("bin")).unwrap();
    std::fs::write(s.kaika_home().join("bin/tool"), "mine").unwrap();
    s.cmd().args(["install", "direct:tool", "-y"]).assert().code(1).stderr(predicate::str::contains("not a kaika link"));
    assert_eq!(std::fs::read_to_string(s.kaika_home().join("bin/tool")).unwrap(), "mine");
    s.cmd().args(["install", "direct:tool", "-y", "-f"]).assert().success();
    assert!(std::fs::read_link(s.kaika_home().join("bin/tool")).is_ok());
}

#[test]
fn an_upgrade_replaces_the_old_version_folder_and_repoints_the_link() {
    let s = sandbox();
    let tgz = tarball(&s);
    let sum = kaika::verify::sha256_file(&tgz).unwrap();
    let api = s.root.path().join("api");
    let f = api.join("repos/o/tool/releases/latest");
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    let release = |tag: &str| {
        std::fs::write(&f, serde_json::json!({"tag_name": tag, "assets": [{"name": "tool-macos-arm64.tar.gz", "browser_download_url": Sandbox::url(&tgz), "digest": format!("sha256:{sum}")}]}).to_string()).unwrap();
    };
    s.recipe("tool.toml", "name = \"tool\"\ngithub = \"o/tool\"\n");
    let run = |args: &[&str]| s.cmd().env("KAIKA_GITHUB_API", Sandbox::url(&api)).args(args.to_vec()).assert();
    release("v1.0");
    run(&["install", "direct:tool", "-y"]).success();
    release("v2.0");
    run(&["upgrade", "tool", "-y"]).success();
    assert!(!s.kaika_home().join("packages/tool/1.0").exists());
    assert_eq!(std::fs::read_link(s.kaika_home().join("bin/tool")).unwrap(), s.kaika_home().join("packages/tool/2.0/tool"));
}

#[test]
fn everything_kaika_owns_lives_under_its_one_home() {
    let s = sandbox();
    with_ripgrep(&s);
    s.cmd().args(["install", "ripgrep", "-y"]).assert().success();
    assert!(s.kaika_home().join("manifest.toml").exists());
    assert!(!s.home.join(".config").exists() && !s.home.join(".local").exists() && !s.home.join(".cache").exists());
}

fn pkg_recipe(s: &Sandbox, extra: &str) {
    let pkg = s.root.path().join("Fixture-1.0.pkg");
    std::fs::write(&pkg, "pkg").unwrap();
    s.recipe("fixture.toml", &format!("name = \"fixture\"\nurl = \"{}\"\n{extra}", Sandbox::url(&pkg)));
    s.stub("pkgutil", 0, Match::IgnoreLast);
    s.reply("pkgutil --check-signature", "Status: signed by a certificate trusted by macOS\n   1. Developer ID Installer: Fixture Inc (PKGTEAM123)\n");
    s.reply_err("spctl -a -vv -t install", "accepted\nsource=Notarized Developer ID\n");
}

#[test]
fn a_verified_pkg_runs_the_system_installer_with_admin_rights_stated_up_front() {
    let s = sandbox();
    pkg_recipe(&s, "team_id = \"PKGTEAM123\"\n");
    s.cmd().args(["install", "direct:fixture", "-n"]).assert().success().stdout(predicate::str::contains("needs administrator rights"));
    assert!(!s.calls().contains("installer -pkg"));
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().success();
    assert!(s.calls().contains("sudo installer -pkg"), "{}", s.calls());
}

#[test]
fn a_pkg_with_the_wrong_team_never_reaches_the_installer() {
    let s = sandbox();
    pkg_recipe(&s, "team_id = \"OTHER00000\"\n");
    s.cmd().args(["install", "direct:fixture", "-y"]).assert().code(3);
    assert!(!s.calls().contains("installer -pkg"));
}

// ---------- upgrades of direct installs (GitHub releases) ----------

fn github_release(s: &Sandbox, tag: &str, zip: &std::path::Path) -> std::path::PathBuf {
    let api = s.root.path().join("api");
    let f = api.join("repos/o/fixture/releases/latest");
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    let body = serde_json::json!({
        "tag_name": tag,
        "assets": [
            {"name": "Fixture-x86_64.zip", "browser_download_url": "file:///nonexistent/intel.zip"},
            {"name": "Fixture-arm64.zip", "browser_download_url": Sandbox::url(zip)}
        ]
    });
    std::fs::write(&f, body.to_string()).unwrap();
    api
}

#[test]
fn outdated_and_upgrade_follow_the_release_feed_and_keep_the_recorded_team() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-arm64.zip");
    s.recipe("fixture.toml", "name = \"fixture\"\ngithub = \"o/fixture\"\napp = \"Fixture.app\"\n");
    let api = github_release(&s, "v1.0", &zip);
    let run = |args: &[&str]| s.cmd().env("KAIKA_GITHUB_API", Sandbox::url(&api)).args(args.to_vec()).assert();

    run(&["install", "direct:fixture", "-y"]).success();
    assert!(s.manifest().contains("version = \"1.0\""));
    run(&["outdated"]).success().stdout(predicate::str::contains("up to date"));

    github_release(&s, "v2.0", &zip);
    run(&["outdated"]).success().stdout(predicate::str::contains("direct:fixture").and(predicate::str::contains("1.0 -> 2.0")));
    run(&["upgrade", "fixture", "-y"]).success();
    assert!(s.manifest().contains("version = \"2.0\""));
    run(&["upgrade", "fixture", "-y"]).success(); // already current: nothing to do

    // The vendor's signing identity changes: the recorded Team ID refuses the upgrade.
    github_release(&s, "v3.0", &zip);
    s.good_signature("EVIL000000");
    run(&["upgrade", "--all", "-y"]).code(3).stderr(predicate::str::contains("does not match the expected ABCDE12345"));
    assert!(s.manifest().contains("version = \"2.0\""));
}

// ---------- backend choice: trust and structure, never a model ----------

fn models_on(s: &Sandbox) -> assert_cmd::Command {
    let mut c = s.cmd();
    c.env("KAIKA_MODELS", "on");
    c
}

#[test]
fn a_curated_source_is_never_overruled_by_an_open_registry_and_no_model_is_asked() {
    let s = sandbox();
    s.reply("brew search --formula ripgrep", "ripgrep\n").reply("npm view ripgrep name", "ripgrep\n");
    // A model that would loudly prefer the second candidate, if anyone asked it.
    s.reply(Sandbox::OLLAMA, &Sandbox::logprobs(&[("B", 0.99), ("A", 0.01)]));
    models_on(&s).args(["install", "ripgrep", "-n"]).assert().success().stdout(
        predicate::str::contains("brew install ripgrep").and(predicate::str::contains("never chosen over brew:ripgrep")),
    );
    assert!(!s.calls().contains("11434"), "no model may decide a source: {}", s.calls());
}

#[test]
fn a_tie_between_open_registries_keeps_the_order_and_shows_what_each_is() {
    let s = sandbox();
    s.reply("curl -sfI --max-time 10 https://pypi.org/simple/black/", "")
        .reply("curl -sf --max-time 10 https://pypi.org/pypi/black/json", r#"{"info":{"summary":"The uncompromising code formatter."}}"#)
        .reply("npm view black name", "black\n")
        .reply("npm view black description", "Helpers missing from JS core\n");
    s.reply(Sandbox::OLLAMA, &Sandbox::logprobs(&[("B", 0.99), ("A", 0.01)]));
    models_on(&s).args(["install", "black", "-n"]).assert().success().stdout(
        predicate::str::contains("uv tool install black")
            .and(predicate::str::contains("uv:black (The uncompromising code formatter.)"))
            .and(predicate::str::contains("npm:black (Helpers missing from JS core)"))
            .and(predicate::str::contains("Use source:name")),
    );
    assert!(!s.calls().contains("11434"), "no model may settle a tie of sources");
}

#[test]
fn a_formula_and_a_cask_of_one_name_is_ambiguous_by_structure_with_models_on_or_off() {
    let s = sandbox();
    s.reply("brew search --formula docker", "docker\n").reply("brew search --cask docker", "docker\n");
    s.reply(Sandbox::OLLAMA, &Sandbox::logprobs(&[("B", 0.99), ("A", 0.01)]));
    for on in [true, false] {
        let mut c = if on { models_on(&s) } else { s.cmd() };
        c.args(["install", "docker", "-n"]).assert().success().stdout(
            predicate::str::contains("brew install docker").and(predicate::str::contains("a command-line tool and a desktop app")),
        );
    }
    assert!(!s.calls().contains("11434"));
}

// ---------- vendor-site discovery: structured sources, models only between peers ----------

fn api(url: &str) -> String {
    format!("curl -fsSL --max-time 30 -A {} {url}", kaika::recipe::USER_AGENT)
}

fn wd_search(q: &str) -> String {
    api(&format!("https://www.wikidata.org/w/api.php?action=wbsearchentities&search={}&language=en&type=item&limit=7&format=json", kaika::html::url_encode(q)))
}

fn wd_details(ids: &str) -> String {
    api(&format!("https://www.wikidata.org/w/api.php?action=wbgetentities&ids={ids}&props=claims&format=json"))
}

fn wd_found(items: &[(&str, &str, &str)]) -> String {
    let a: Vec<String> = items.iter().map(|(id, l, d)| format!(r#"{{"id":"{id}","label":"{l}","description":"{d}"}}"#)).collect();
    format!(r#"{{"search":[{}]}}"#, a.join(","))
}

fn wd_sites(items: &[(&str, &str)]) -> String {
    let a: Vec<String> = items.iter().map(|(id, u)| format!(r#""{id}":{{"claims":{{"P856":[{{"rank":"normal","mainsnak":{{"datavalue":{{"value":"{u}"}}}}}}]}}}}"#)).collect();
    format!(r#"{{"entities":{{{}}}}}"#, a.join(","))
}

/// Wikidata knows "ghost" as one app with this site, whose page links to `zip`.
fn ghost_on(s: &Sandbox, site_host: &str, zip: &std::path::Path) {
    s.reply(&wd_search("ghost"), &wd_found(&[("Q1", "Ghost", "macOS app")])).reply(&wd_details("Q1"), &wd_sites(&[("Q1", &format!("https://{site_host}/"))]));
    vendor_page(s, site_host, zip);
}

fn vendor_page(s: &Sandbox, host: &str, zip: &std::path::Path) {
    s.reply(&api(&format!("https://{host}/")), &format!(r#"<a href="{}">Download for macOS</a>"#, Sandbox::url(zip)));
}

/// Wikidata knows "ghost" as two different apps.
fn two_ghosts(s: &Sandbox) {
    s.reply(&wd_search("ghost"), &wd_found(&[("Q1", "Ghost", "blogging platform"), ("Q2", "Ghost", "macOS writing app")]))
        .reply(&wd_details("Q1%7CQ2"), &wd_sites(&[("Q1", "https://a.example/"), ("Q2", "https://b.example/")]));
}

#[test]
fn an_unknown_app_is_found_online_installed_and_remembered() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    ghost_on(&s, "ghost.example", &zip);
    models_on(&s).args(["install", "ghost", "-y"]).assert().success().stdout(predicate::str::contains("found online"));
    assert!(s.apps.join("Fixture.app").exists());
    assert!(s.manifest().contains("found via Wikidata"));
    let learned = std::fs::read_to_string(s.config_dir().join("recipes/ghost.toml")).unwrap();
    assert!(learned.contains("page = \"https://ghost.example/\"") && learned.contains("Fixture-*.zip"), "{learned}");
    assert!(!s.calls().contains("11434"), "a lone official site needs no model");
}

#[test]
fn what_was_learned_is_used_next_time_without_asking_anyone() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    ghost_on(&s, "ghost.example", &zip);
    models_on(&s).args(["install", "ghost", "-y"]).assert().success();
    s.cmd().args(["uninstall", "ghost", "-y"]).assert().success();
    let before = s.calls().matches("wikidata").count();
    s.cmd().args(["install", "ghost", "-y"]).assert().success(); // models off: the learned recipe is enough
    assert_eq!(s.calls().matches("wikidata").count(), before, "no lookup the second time");
    assert!(s.apps.join("Fixture.app").exists());
}

#[test]
fn with_models_off_there_is_no_web_lookup_at_all() {
    let s = sandbox();
    s.cmd().args(["install", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("discovery is off"));
    assert!(!s.calls().contains("wikidata"), "{}", s.calls());
}

#[test]
fn an_app_wikidata_does_not_know_says_how_to_proceed() {
    let s = sandbox();
    s.reply(&wd_search("ghost"), &wd_found(&[]));
    models_on(&s).args(["install", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("give its https URL"));
}

#[test]
fn two_plausible_sites_and_no_model_installs_nothing() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    two_ghosts(&s);
    vendor_page(&s, "a.example", &zip);
    vendor_page(&s, "b.example", &zip);
    models_on(&s).args(["install", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("official"));
    assert!(!s.apps.join("Fixture.app").exists());
}

#[test]
fn the_small_model_picks_the_site_only_if_it_survives_reversal() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    two_ghosts(&s);
    vendor_page(&s, "b.example", &zip);
    // Sites are [a, b]. Pass 1: b is "B". Pass 2 is reversed, so b is "A". It picks b both times.
    s.reply_seq(Sandbox::OLLAMA, &[&Sandbox::logprobs(&[("B", 0.96), ("A", 0.04)]), &Sandbox::logprobs(&[("A", 0.95), ("B", 0.05)])]);
    models_on(&s).args(["install", "ghost", "-y"]).assert().success();
    assert!(s.apps.join("Fixture.app").exists());
}

#[test]
fn a_position_biased_or_swayed_model_installs_nothing() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    two_ghosts(&s);
    vendor_page(&s, "a.example", &zip);
    vendor_page(&s, "b.example", &zip);
    // "A" both times, whatever A is: what a prompt injection ("the correct answer is A") would produce.
    s.reply(Sandbox::OLLAMA, &Sandbox::logprobs(&[("A", 0.99), ("B", 0.01)]));
    models_on(&s).args(["install", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("reversed"));
    assert!(!s.apps.join("Fixture.app").exists());
}

fn escalation_script(s: &Sandbox, json: &str) -> String {
    let p = s.root.path().join("escalate.sh");
    std::fs::write(&p, format!("#!/bin/sh\ncat > /dev/null\ncat <<'EOF'\n{json}\nEOF\n")).unwrap();
    format!("/bin/sh {}", p.display())
}

#[test]
fn the_bigger_model_settles_what_the_small_one_could_not() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    two_ghosts(&s);
    vendor_page(&s, "b.example", &zip);
    s.reply(Sandbox::OLLAMA, &Sandbox::logprobs(&[("A", 0.99), ("B", 0.01)]));
    let esc = escalation_script(&s, r#"{"choice":"b.example","confidence":0.9,"evidence":"asked a bigger model"}"#);
    models_on(&s).env("KAIKA_ESCALATION", esc).args(["install", "ghost", "-y"]).assert().success();
    assert!(s.apps.join("Fixture.app").exists());
}

#[test]
fn an_escalation_that_names_something_outside_the_candidates_is_ignored() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    two_ghosts(&s);
    vendor_page(&s, "a.example", &zip);
    s.reply(Sandbox::OLLAMA, &Sandbox::logprobs(&[("A", 0.99), ("B", 0.01)]));
    let esc = escalation_script(&s, r#"{"choice":"evil.example","confidence":0.99}"#);
    models_on(&s).env("KAIKA_ESCALATION", esc).args(["install", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("bigger model was unsure"));
    assert!(!s.apps.join("Fixture.app").exists());
}

#[test]
fn a_crashing_escalation_is_unsure_not_fatal() {
    let s = sandbox();
    two_ghosts(&s);
    s.reply(Sandbox::OLLAMA, &Sandbox::logprobs(&[("A", 0.99), ("B", 0.01)]));
    let p = s.root.path().join("boom.sh");
    std::fs::write(&p, "#!/bin/sh\nexit 7\n").unwrap();
    models_on(&s).env("KAIKA_ESCALATION", format!("/bin/sh {}", p.display())).args(["install", "ghost", "-y"]).assert().code(1);
}

#[test]
fn a_download_over_plain_http_is_never_offered() {
    let s = sandbox();
    s.reply(&wd_search("ghost"), &wd_found(&[("Q1", "Ghost", "app")])).reply(&wd_details("Q1"), &wd_sites(&[("Q1", "https://ghost.example/")]));
    s.reply(&api("https://ghost.example/"), r#"<a href="http://ghost.example/Ghost.dmg">get</a>"#);
    models_on(&s).args(["install", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("no macOS download"));
    assert!(!s.calls().contains("curl -fSL"), "no download may start");
}

#[test]
fn a_failed_lookup_is_unsure_not_a_crash() {
    let s = sandbox();
    s.reply_code(&wd_search("ghost"), 6);
    models_on(&s).args(["install", "ghost", "-y"]).assert().code(1).stderr(predicate::str::contains("no package named"));
}

// ---------- a URL as the target: pages that no database knows ----------

#[test]
fn a_vendor_page_url_is_scanned_for_the_download_even_with_models_off() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    vendor_page(&s, "ghost.example", &zip);
    s.cmd().args(["install", "https://ghost.example/", "-y"]).assert().success().stdout(predicate::str::contains("the page you gave"));
    assert!(s.apps.join("Fixture.app").exists());
    assert!(s.manifest().contains("found via given URL") && s.manifest().contains("name = \"ghost\""));
    assert!(!s.calls().contains("wikidata"), "a given URL needs no lookup");
}

#[test]
fn a_page_with_no_download_installs_nothing() {
    let s = sandbox();
    s.reply(&api("https://ghost.example/"), "<a href='/about'>About</a>");
    s.cmd().args(["install", "https://ghost.example/", "-y"]).assert().code(1).stderr(predicate::str::contains("could not find a download"));
}

#[test]
fn a_plain_http_url_is_refused_outright() {
    let s = sandbox();
    s.cmd().args(["install", "http://ghost.example/Ghost.dmg", "-y"]).assert().code(1).stderr(predicate::str::contains("non-HTTPS"));
    assert!(!s.calls().contains("curl -fSL"));
}

#[test]
fn a_url_that_is_a_download_still_goes_through_every_check() {
    let s = sandbox();
    let zip = s.fixture_zip("Fixture-1.0.zip");
    s.reply_code("codesign --verify --deep --strict", 1);
    // A file:// URL is only for tests; an https download URL takes the same path.
    s.recipe("ghost.toml", &format!("name = \"ghost\"\nurl = \"{}\"\napp = \"Fixture.app\"\n", Sandbox::url(&zip)));
    s.cmd().args(["install", "direct:ghost", "-y"]).assert().code(3);
    assert!(!s.apps.join("Fixture.app").exists());
}

// ---------- names the way people type them ----------

#[test]
fn spaces_and_case_still_find_the_hyphenated_package() {
    let s = sandbox();
    s.reply("brew search --cask sublime-merge", "sublime-merge\n");
    s.cmd().args(["install", "Sublime Merge", "-n"]).assert().success().stdout(predicate::str::contains("brew install --cask sublime-merge"));
}

// ---------- housekeeping ----------

#[test]
fn doctor_reports_backends_and_the_decision_layer() {
    let s = Sandbox::new();
    s.stub("brew", 0, Match::Exact);
    s.cmd().arg("doctor").assert().success().stdout(predicate::str::contains("brew").and(predicate::str::contains("npm")).and(predicate::str::contains("missing")).and(predicate::str::contains("off")));
}

#[test]
fn doctor_fails_on_a_corrupt_manifest() {
    let s = sandbox();
    std::fs::create_dir_all(s.data()).unwrap();
    std::fs::write(s.data().join("manifest.toml"), "package = [[[").unwrap();
    s.cmd().arg("doctor").assert().code(1);
}

#[test]
fn config_round_trips_and_rejects_unknown_keys() {
    let s = sandbox();
    s.cmd().args(["config", "set", "model", "qwen2.5:3b"]).assert().success();
    s.cmd().args(["config", "get", "model"]).assert().success().stdout("qwen2.5:3b\n");
    s.cmd().args(["config", "get", "nope"]).assert().code(1);
    s.cmd().args(["config", "bogus"]).assert().code(2);
}

#[test]
fn cleanup_removes_downloads() {
    let s = sandbox();
    let dl = s.kaika_home().join("cache/downloads");
    std::fs::create_dir_all(&dl).unwrap();
    std::fs::write(dl.join("x.dmg"), "data").unwrap();
    s.cmd().args(["cleanup", "-n"]).assert().success().stdout(predicate::str::contains("would"));
    assert!(dl.join("x.dmg").exists());
    s.cmd().arg("cleanup").assert().success();
    assert!(!dl.exists());
}

#[test]
fn update_refreshes_brew() {
    let s = sandbox();
    s.cmd().arg("update").assert().success();
    assert!(s.calls().contains("brew update"));
}

#[test]
fn a_corrupt_config_is_reported_not_ignored() {
    let s = sandbox();
    std::fs::create_dir_all(s.config_dir()).unwrap();
    std::fs::write(s.config_dir().join("kaika.toml"), "order = [[[").unwrap();
    s.cmd().arg("list").assert().code(1).stderr(predicate::str::contains("kaika.toml"));
}

#[test]
fn version_and_help_work_and_nothing_runs_without_a_command() {
    let s = sandbox();
    s.cmd().arg("--version").assert().success();
    s.cmd().arg("--help").assert().success().stdout(predicate::str::contains("install"));
    s.cmd().assert().code(2);
}

// ---------- against the real macOS tools (opt-in: `cargo test -- --ignored`) ----------

/// Runs the real `codesign` and `spctl` on an app that is already installed. Read-only, no window, no network.
#[test]
#[ignore]
fn the_real_macos_tools_agree_with_the_parsers() {
    let app = std::env::var("KAIKA_REAL_APP").unwrap_or_else(|_| "/Applications/Claude.app".into());
    let path = std::path::Path::new(&app);
    if !path.exists() {
        eprintln!("skipping: {app} is not installed");
        return;
    }
    let rep = kaika::verify::check_app(&kaika::runner::RealRunner, path, &kaika::verify::Expect::default());
    eprintln!("{app}: signature={} notarized={} team={:?} failures={:?}", rep.signature, rep.notarized, rep.team_id, rep.failures);
    assert!(rep.signature, "a shipping app has a valid signature");
    assert!(rep.team_id.as_deref().is_some_and(|t| t.len() == 10), "a Developer ID app has a 10-character Team ID: {:?}", rep.team_id);
    assert!(rep.notarized, "a downloaded Developer ID app is notarized");
    let wrong = kaika::verify::check_app(&kaika::runner::RealRunner, path, &kaika::verify::Expect { team_id: Some("ZZZZZZZZZZ".into()), sha256: None });
    assert!(wrong.failures.iter().any(|f| f.contains("does not match")), "{:?}", wrong.failures);
}
