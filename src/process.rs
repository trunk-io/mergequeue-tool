use std::process::Command;

fn exec(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .expect(&format!("Failed to execute {}", cmd));

    if !output.status.success() {
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        eprintln!("Call to {} {} failed", cmd, args.join(" "));
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    } else {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
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
