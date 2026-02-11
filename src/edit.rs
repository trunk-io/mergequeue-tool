use crate::config::Conf;
use rand::Rng;
use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::io::{BufReader, BufWriter};

/// Parse a line as `{word}` or `{word} {integer}`.
/// Returns `Some((word, count))` where count is 0 when no integer is present.
/// Returns `None` if the line is not in that form (e.g. empty, or "word x y", or "word not_a_number").
fn parse_line(line: &str) -> Option<(String, u32)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    match parts.len() {
        1 => Some((parts[0].to_string(), 0)),
        2 => parts[1]
            .parse::<u32>()
            .ok()
            .map(|n| (parts[0].to_string(), n)),
        _ => None,
    }
}

pub fn change_file(filenames: &[String], count: u32) -> Vec<String> {
    let mut rng = rand::thread_rng();
    let mut words: Vec<String> = Vec::new();

    if count > filenames.len() as u32 {
        panic!("The count must be less than the number of files");
    }

    let mut indices: Vec<usize> = (0..filenames.len()).collect();
    for i in 0..count as usize {
        let j = rng.gen_range(i..indices.len());
        indices.swap(i, j);
    }

    for i in 0..count as usize {
        let filename = &filenames[indices[i]];
        words.push(edit_random_line(filename));
    }

    words
}

/// Pick a random line in the file. If it matches `{word}` or `{word} {integer}`,
/// update it to `{word} {integer+1}`. If not, delete that line and try another until we edit one.
/// Returns the word that was edited.
fn edit_random_line(filename: &str) -> String {
    let file = std::fs::File::open(filename).expect("Failed to open file");
    let reader = BufReader::new(file);
    let mut lines: Vec<String> = reader
        .lines()
        .collect::<Result<_, _>>()
        .expect("failed to read lines");

    if lines.is_empty() {
        panic!("Cannot continue the file {} is empty", filename);
    }

    let mut rng = rand::thread_rng();

    while !lines.is_empty() {
        let line_index = rng.gen_range(0..lines.len());
        let line = lines[line_index].clone();
        let trimmed = line.trim();

        if let Some((word, n)) = parse_line(trimmed) {
            // Valid: replace with {word} {n+1}
            lines[line_index] = format!("{} {}", word, n + 1);
            write_lines(filename, &lines);
            return word.to_lowercase();
        }

        // Not in expected form: delete this line from the file
        lines.remove(line_index);
        write_lines(filename, &lines);
    }

    panic!(
        "No valid line (format '{{word}}' or '{{word}} {{integer}}') left in file {}",
        filename
    );
}

fn write_lines(filename: &str, lines: &[String]) {
    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(filename)
        .expect("failed to open file for write");
    let mut writer = BufWriter::new(file);
    for line in lines {
        writeln!(writer, "{}", line).expect("failed to write file");
    }
}

/// Edit files for a PR based on the configuration and PR number.
/// Returns the words that were changed in the files.
pub fn edit_files_for_pr(filenames: &[String], pr_number: u32, config: &Conf) -> Vec<String> {
    let (selected_files, change_count) = if config.pullrequest.deps_distribution.is_some() {
        let dependency_count = config.get_dependency_count(pr_number, filenames.len());
        (filenames.to_vec(), dependency_count)
    } else {
        let max_files = config.pullrequest.max_deps.min(filenames.len());
        let files: Vec<String> = filenames.iter().take(max_files).cloned().collect();
        (files, config.pullrequest.max_impacted_deps)
    };

    change_file(&selected_files, change_count as u32)
}

#[cfg(test)]
mod tests {
    use super::parse_line;
    use std::fs;

    #[test]
    fn test_parse_line_word_only() {
        assert_eq!(parse_line("died"), Some(("died".into(), 0)));
        assert_eq!(parse_line("  alpha  "), Some(("alpha".into(), 0)));
    }

    #[test]
    fn test_parse_line_word_and_integer() {
        assert_eq!(parse_line("died 9"), Some(("died".into(), 9)));
        assert_eq!(parse_line("died 1"), Some(("died".into(), 1)));
        assert_eq!(parse_line("  word  42  "), Some(("word".into(), 42)));
    }

    #[test]
    fn test_parse_line_invalid() {
        assert_eq!(parse_line(""), None);
        assert_eq!(parse_line("   "), None);
        assert_eq!(parse_line("a b c"), None);
        assert_eq!(parse_line("word abc"), None);
        assert_eq!(parse_line("word -1"), None);
    }

    #[test]
    fn test_edit_increments_and_writes() {
        let dir = std::env::temp_dir().join("mq_edit_test");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("f.txt");
        fs::write(&path, "one\ntwo 3\nthree\n").unwrap();

        let word = super::edit_random_line(path.to_str().unwrap());
        let content = fs::read_to_string(&path).unwrap();
        // One of the valid lines was edited: "one" -> "one 1", or "two 3" -> "two 4", or "three" -> "three 1"
        assert!(
            content.contains("one 1") || content.contains("two 4") || content.contains("three 1")
        );
        assert!(["one", "two", "three"].contains(&word.as_str()));

        let _ = fs::remove_dir_all(&dir);
    }
}
