use std::process::Command;

fn exec(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .expect(&format!("Failed to execute {}", cmd));

    if !output.status.success() {
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        eprintln!("Call to {} {} failed", cmd, args.join(" "));
        return Err(String::from_utf8_lossy(&output.stderr)
            .into_owned()
            .trim()
            .to_string());
    } else {
        return Ok(String::from_utf8_lossy(&output.stdout)
            .into_owned()
            .trim()
            .to_string());
    }
}

pub fn run_cmd(cmd: &str) -> String {
    let args: Vec<&str> = cmd.split_whitespace().collect();
    exec(args.first().unwrap(), &args[1..]).expect("run failed")
}

pub fn gh(args: &[&str]) -> String {
    exec("gh", args).expect("gh exec failed")
}

pub fn try_gh(args: &[&str]) -> Result<String, String> {
    exec("gh", args)
}

pub fn git(args: &[&str]) -> String {
    exec("git", args).expect("git exec failed")
}

pub fn try_git(args: &[&str]) -> Result<String, String> {
    exec("git", args)
}
