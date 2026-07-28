#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::process::Command;

    fn write_fake_asana_client(bin: &Path) -> std::path::PathBuf {
        let client = bin.join("asana");
        fs::write(
            &client,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$ASANA_LOG"
cat > "$ASANA_STDIN"
printf '%s\n' '{"data":{"gid":"task-123","permalink_url":"https://app.asana.com/0/project/task-123"}}'
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&client).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&client, permissions).unwrap();
        client
    }

    #[test]
    fn creates_a_demo_asana_task() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        let log = temp.path().join("asana.log");
        let stdin = temp.path().join("asana.stdin");
        fs::create_dir(&bin).unwrap();
        let client = write_fake_asana_client(&bin);
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/create-demo-issue.sh");

        let output = Command::new(script)
            .arg("A rough idea")
            .arg("Please turn this into a task.")
            .env("FLASHY_FACTORY_ASANA_CLIENT", client)
            .env("ASANA_LOG", &log)
            .env("ASANA_STDIN", &stdin)
            .output()
            .unwrap();

        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("https://app.asana.com/0/project/task-123"));
        assert!(stdout.contains("Ready For Spec"));
        assert!(stdout.contains("cargo run -- run"));

        let calls = fs::read_to_string(log).unwrap();
        assert!(calls.contains("create --name A rough idea"));
        assert!(calls.contains("--section Ready For Spec"));
        assert_eq!(
            fs::read_to_string(stdin).unwrap(),
            "Please turn this into a task.\n"
        );
    }

    #[test]
    fn rejects_a_missing_idea_without_calling_asana() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/create-demo-issue.sh");

        let output = Command::new(script).output().unwrap();

        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Usage:"));
    }
}
