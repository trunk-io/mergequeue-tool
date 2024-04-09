use rand::Rng;
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::io::{BufReader, BufWriter};

pub fn change_file(filenames: &[String], count: u32) -> Vec<String> {
    let mut rng = rand::thread_rng();
    let mut unique_filenames: HashSet<String> = HashSet::new();
    let mut words: Vec<String> = Vec::new();

    if count > filenames.len() as u32 {
        panic!("The count must be less than the number of files");
    }

    for _ in 0..count {
        let idx = rng.gen_range(0..filenames.len());
        let filename = &filenames[idx];
        if unique_filenames.contains(filename) {
            continue;
        }
        words.push(move_random_line(filename));
        unique_filenames.insert(filename.to_string());
    }

    words
}

pub fn move_random_line(filename: &str) -> String {
    // Read the file into a vector of lines
    let file = std::fs::File::open(&filename).expect("Failed to open file");
    let reader = BufReader::new(file);
    let mut lines: Vec<String> = reader
        .lines()
        .collect::<Result<_, _>>()
        .expect("failed to read lines");

    if lines.is_empty() {
        panic!("Cannot continue the file {} is empty", filename);
    }

    // Choose a random line
    let mut rng = rand::thread_rng();
    let line_index = rng.gen_range(0..lines.len());

    // Remove the line from the vector
    let line = lines.remove(line_index);
    let word = line.trim().to_string();

    // Choose another random line
    let other_line_index = rng.gen_range(0..lines.len());

    // Insert the line at the new position
    lines.insert(other_line_index, line);

    // Write the lines back to the file
    let file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&filename)
        .expect("failed to open file");
    let mut writer = BufWriter::new(file);
    for line in lines {
        writeln!(writer, "{}", line).expect("failed to write file");
    }

    word.to_lowercase().to_string()
}
