//! M2.3, a new film round trip: a project made with New project is laid out
//! the way Last order is, by Spumcake-Skills' own `request.py init`; its first
//! request is made in Studio, through the host's runner; it is carried on
//! with Skills' scripts outside Studio, a request and a publish; and Skills'
//! own check accepts all of it, with git showing only what the steps made.
//! The Project tab's view of both requests follows the approved views.
//!
//! Runs where SPUMCAKE_SKILLS_SCRIPTS names a Spumcake-Skills checkout's
//! scripts, as check.sh sets; every request runs on `request.py`'s
//! stand-in, which spends nothing.

#[cfg(test)]
mod tests {
    use crate::runner::{Begun, Runner, SEND_REQUEST, ScriptEnv};
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Skills' scripts, run as any agent runs them, with no Studio.
    fn harness(scripts: &Path, folder: &Path, script: &str, args: &[&str]) -> String {
        let out = Command::new("python3").arg(scripts.join(script)).args(args).current_dir(folder).output().unwrap();
        assert!(out.status.success(), "{script} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// Runs a request through the host's runner, as a cell's Generate does:
    /// asked with its plan, approved, and waited for.
    fn studio_request(scripts: &Path, folder: &Path, spec: &str) {
        let mut runner = Runner::default();
        runner.assume_prerequisites(folder);
        let env = ScriptEnv { scripts: Some(scripts.to_path_buf()), ..ScriptEnv::default() };
        if let Begun::Answered(result) = runner.begin(folder, env, "cell:first", "", SEND_REQUEST, &json!({ "spec": spec, "stand_in": true })) {
            panic!("answered before it was asked: {:?}", result.output);
        }
        let start = std::time::Instant::now();
        while runner.asking().is_none() {
            assert!(runner.poll().0.is_empty(), "answered before it was asked");
            assert!(start.elapsed().as_secs() < 30, "the plan never came");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(runner.answer(true).is_none());
        let answer = loop {
            if let Some(answer) = runner.poll().0.pop() {
                break answer;
            }
            assert!(start.elapsed().as_secs() < 90, "request.py never ended");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        assert!(answer.ok, "Studio's request failed: {:?}", answer.output);
    }

    fn files(folder: &Path, sub: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder.join(sub))
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n != ".gitkeep").collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn a_new_film_moves_between_studio_and_a_plain_harness() {
        let Some(scripts) = std::env::var_os("SPUMCAKE_SKILLS_SCRIPTS").map(PathBuf::from) else {
            eprintln!("SPUMCAKE_SKILLS_SCRIPTS is not set, so the new film's round trip is not checked");
            return;
        };
        let Some(checkout) = scripts.parent().map(Path::to_path_buf) else { return };
        let root = std::env::temp_dir().join(format!("new-film-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        let folder = root.join("The Last Signal");

        // 1. New project, as the window makes one: Skills lays it out, and
        // Studio adds only its own folder.
        let skills = checkout.join("skills").display().to_string();
        crate::workspace::create_laid_out_project(&root.join("data"), &folder, "The Last Signal", |f| {
            crate::skills_checkout::lay_out(f, std::slice::from_ref(&skills))
        })
        .expect("New project made the film");
        for made in ["brief", "library", "requests", "prompts", "pipeline/001/canon.sql", "agent/index.md", "work", protocol::PROJECT_DIR] {
            assert!(folder.join(made).exists(), "the new film has no {made}");
        }
        // `request.py init` makes a repository only for a folder in none.
        // Tests run inside Studio's own tree, whose `target/` git ignores, so
        // the film is given the repository it would have anywhere else, and
        // git sees the film alone.
        let top = Command::new("git").args(["rev-parse", "--show-toplevel"]).current_dir(&folder).output().unwrap();
        if !top.status.success() || Path::new(String::from_utf8_lossy(&top.stdout).trim()) != folder {
            assert!(Command::new("git").args(["init", "-q"]).current_dir(&folder).status().unwrap().success());
        }
        // What New project left, as git sees it, is the baseline.
        let baseline = git_status(&folder);
        assert!(!baseline.is_empty(), "git sees nothing of the new film");

        // 2. Its first request, made in Studio through the host's runner.
        std::fs::write(
            folder.join("agent/2026-11-18_10-00-spec-keeper.json"),
            r#"{"name": "keeper-sheet", "skill": "generate-model-sheet", "asked": "The keeper's first sheet.", "why": "The film's first character.", "endpoint": {"kind": "fal", "model": "stand-in"}, "prompt": "A lighthouse keeper, front and side."}"#,
        )
        .unwrap();
        studio_request(&scripts, &folder, "agent/2026-11-18_10-00-spec-keeper.json");
        let after_studio = files(&folder, "requests");
        assert_eq!(after_studio.len(), 1, "{after_studio:?}");
        let studio_id = after_studio[0].trim_end_matches(".html").to_string();

        // 3. Carried on outside Studio: a request and a publish, with Skills'
        // scripts alone, and Studio's own folder gone, as a harness has none.
        let kept = root.join("kept-spumcake");
        std::fs::rename(folder.join(protocol::PROJECT_DIR), &kept).unwrap();
        std::fs::write(
            folder.join("agent/2026-11-18_10-05-spec-light.json"),
            r#"{"name": "storm-light", "skill": "generate-concept-artwork", "endpoint": {"kind": "fal", "model": "stand-in"}, "prompt": "The lighthouse in a storm, at night."}"#,
        )
        .unwrap();
        let plan: Value = serde_json::from_str(&harness(&scripts, &folder, "request.py", &["plan", "agent/2026-11-18_10-05-spec-light.json", "--json"])).unwrap();
        harness(&scripts, &folder, "request.py", &["run", "agent/2026-11-18_10-05-spec-light.json", "--stand-in", "--expect", plan["hash"].as_str().unwrap()]);
        let harness_id = files(&folder, "requests").into_iter().find(|n| !n.starts_with(&studio_id)).unwrap().trim_end_matches(".html").to_string();
        let output = files(&folder, "library").into_iter().find(|n| n.starts_with(&harness_id)).expect("the harness's request filed nothing");
        harness(&scripts, &folder, "pipeline.py", &["element", "001", "the keeper", "--kind", "character"]);
        harness(&scripts, &folder, "pipeline.py", &["chain", "001", "the keeper", "--order", "01"]);
        harness(
            &scripts,
            &folder,
            "pipeline.py",
            &["publish", "001", "the keeper: model sheet", "--file", &format!("library/{output}"), "--request", &harness_id, "--chain", "the keeper"],
        );

        // 4. Back in Studio: its folder returns, and both requests, both
        // outputs, and the canon are there for the Project tab to show.
        std::fs::rename(&kept, folder.join(protocol::PROJECT_DIR)).unwrap();
        let requests = files(&folder, "requests");
        assert_eq!(requests.len(), 2, "{requests:?}");
        assert!(requests.iter().any(|n| n.starts_with(&studio_id)) && requests.iter().any(|n| n.starts_with(&harness_id)));
        for id in [&studio_id, &harness_id] {
            assert!(files(&folder, "library").iter().any(|n| n.starts_with(id.as_str())), "{id} filed no output");
            harness(&scripts, &folder, "request.py", &["tell", id]);
        }
        let canon = harness(&scripts, &folder, "pipeline.py", &["sql", "001", "SELECT file FROM artifact WHERE id = 'the keeper: model sheet'", "--read-only"]);
        assert!(canon.contains(&output), "the published sheet is not canon: {canon}");

        // 5. Skills' own check, and git: nothing but what the steps made.
        harness(&scripts, &folder, "request.py", &["check"]);
        let changed: Vec<String> = git_status(&folder).into_iter().filter(|line| !baseline.contains(line)).collect();
        assert!(changed.len() >= 6, "git shows too little of what the steps made: {changed:?}");
        for line in &changed {
            let path = line.get(3..).unwrap_or_default();
            let made = ["requests/", "library/", "prompts/", "agent/", "pipeline/001/"].iter().any(|from| path.starts_with(from));
            assert!(made, "git shows a change no step made: {line}");
        }
        eprintln!("a new film moved between Studio and Skills' scripts: {} requests, {} git changes past New project", requests.len(), changed.len());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `git status --porcelain --untracked-files=all`, one line a path.
    fn git_status(folder: &Path) -> Vec<String> {
        let out = Command::new("git").args(["status", "--porcelain", "--untracked-files=all"]).current_dir(folder).output().unwrap();
        assert!(out.status.success(), "git status: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect()
    }
}
