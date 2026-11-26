//! The project encoding is lossy. Refuse a candidate whose recorded cwd contradicts the tab.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

const HEAD: u64 = 64 * 1024;

fn normalized(cwd: &str) -> String {
    let path = cwd.replace('\\', "/");
    let path = path.strip_prefix("//?/").unwrap_or(&path);
    let path = path.trim_end_matches('/');
    if cfg!(windows) { path.to_lowercase() } else { path.to_owned() }
}

pub fn cwd_matches(path: &Path, cwd: &Path) -> bool {
    let Ok(file) = File::open(path) else { return false };
    check(BufReader::new(file.take(HEAD)), &normalized(&cwd.to_string_lossy()))
}

fn check(reader: impl BufRead, cwd: &str) -> bool {
    for line in reader.split(b'\n').map_while(Result::ok) {
        if let Ok(record) = serde_json::from_slice::<serde_json::Value>(&line)
            && let Some(found) = record["cwd"].as_str()
            && !found.is_empty()
        {
            return normalized(found) == cwd;
        }
    }
    // A Claude file might not have written a complete message yet. The creation time and
    // encoded project directory are still the best available match.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lossy_folder_collision_does_not_match_a_different_recorded_cwd() {
        let data = b"{\"type\":\"system\"}\n{\"cwd\":\"C:/work/my.folder\"}\n";
        assert!(check(data.as_slice(), &normalized(r"C:\work\my.folder")));
        assert!(!check(data.as_slice(), &normalized(r"C:\work\my-folder")));
        assert!(check(b"{\"type\":\"system\"}\n".as_slice(), &normalized(r"C:\work\my-folder")));
    }
}
