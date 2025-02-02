use tracing::{error, info};
use std::path::PathBuf;

use tokio::io::AsyncWriteExt;
use tokio::io::AsyncReadExt;

use similar::{ChangeTag, TextDiff};


pub async fn telemetry_storage_dirs(cache_dir: &PathBuf) -> (PathBuf, PathBuf) {
    let dir = cache_dir.join("telemetry").join("compressed");
    tokio::fs::create_dir_all(dir.clone()).await.unwrap_or_else(|_| {});
    let dir2 = cache_dir.join("telemetry").join("sent");
    tokio::fs::create_dir_all(dir2.clone()).await.unwrap_or_else(|_| {});
    (dir, dir2)
}

pub fn get_add_del_from_texts(
    text_a: &String,
    text_b: &String,
) -> (String, String) {
    let diff = TextDiff::from_lines(text_a, text_b);
    let mut added = "".to_string();
    let mut removed = "".to_string();
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {
                removed += change.value();
            }
            ChangeTag::Insert => {
                added += change.value();
            }
            ChangeTag::Equal => {
            }
        }
    }
    added = added.replace("\r", "");
    removed = added.replace("\r", "");
    (added, removed)
}


pub async fn file_save(path: PathBuf, json: serde_json::Value) -> Result<(), String> {
    let mut f = tokio::fs::File::create(path).await.map_err(|e| format!("{:?}", e))?;
    f.write_all(serde_json::to_string_pretty(&json).unwrap().as_bytes()).await.map_err(|e| format!("{}", e))?;
    Ok(())
}

pub async fn cleanup_old_files(
    dir: PathBuf,
    how_much_to_keep: i32,
) {
    let files = sorted_json_files(dir.clone()).await;
    let mut leave_alone = how_much_to_keep;
    for path in files {
        leave_alone -= 1;
        if leave_alone > 0 {
            // info!("leave_alone telemetry file: {}", path.to_str().unwrap());
            continue;
        }
        info!("removing old telemetry file: {}", path.to_str().unwrap());
        tokio::fs::remove_file(path).await.unwrap_or_else(|e| {
            error!("error removing old telemetry file: {}", e);
            // better to continue deleting, not much we can do
        });
    }
}

pub async fn sorted_json_files(dir: PathBuf) -> Vec<PathBuf> {
    // Most recent files first
    if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
        let mut sorted = Vec::<PathBuf>::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            if !entry.file_type().await.unwrap().is_file() {
                continue;
            }
            let path = entry.path();
            if !path.to_str().unwrap().ends_with(".json") {
                continue;
            }
            sorted.push(path);
        }
        sorted.sort_by(|a, b| b.cmp(&a));
        sorted
    } else {
        Vec::<PathBuf>::new()
    }
}

pub async fn read_file(path: PathBuf) -> Result<String, String> {
    let mut f = tokio::fs::File::open(path.clone()).await.map_err(|e| format!("{:?}", e))?;
    let mut contents = String::new();
    f.read_to_string(&mut contents).await.map_err(|e| format!("{}", e))?;
    Ok(contents)
}

pub fn extract_extension_or_filename(uri: &str) -> String {
    // https://example.com/path/to/file.txt -> .txt
    // https://example.com/path/to/file_without_extension -> file_without_extension
    let parts: Vec<&str> = uri.split('/').collect();
    let last_part = parts.last().unwrap_or(&"");

    if let Some(dot_idx) = last_part.rfind('.') {
        last_part[dot_idx..].to_string()
    } else {
        last_part.to_string()
    }
}

pub fn if_head_tail_equal_return_added_text(
    text_a: &String,
    text_b: &String,
    orig_grey_text: &String,
) -> (bool, String) {
    // params:
    // text_a -- initial file state captured when completion was proposed as a grey text
    // text_b -- file state after user edited it
    // orig_grey_text -- original grey text of completion, initially proposed by a model
    // return: tuple of:
    // bool -- whether diff represents completion (true) or user did modifications that are no longer considered as a completion (false)
    // String -- modified by user completion text
    let diff = TextDiff::from_lines(text_a, text_b);
    let mut allow_add_spaces_once = true;
    let is_multiline = orig_grey_text.contains("\n");
    let mut adding_one_block = false;
    let mut added_one_block = false;
    let mut added_text = "".to_string();
    let mut kill_slash_n = false;
    let regex_space_only = regex::Regex::new(r"^\s*$").unwrap();
    let mut deletion_once = "".to_string();
    for c in diff.iter_all_changes() {
        match c.tag() {
            ChangeTag::Delete => {
                // info!("- {}", c.value());
                if adding_one_block {
                    added_one_block = true;
                }
                let whitespace_only = regex_space_only.is_match(&c.value());
                if !whitespace_only {
                    if deletion_once.is_empty() {
                        deletion_once = c.value().clone().to_string();
                        if deletion_once.ends_with("\n") {
                            deletion_once = deletion_once[..deletion_once.len() - 1].to_string();
                        }
                    } else {
                        // error!("!whitespace_only");
                        return (false, "".to_string());
                    }
                }
                if c.value().ends_with("\n") {
                    kill_slash_n = true;
                }
            }
            ChangeTag::Insert => {
                // info!("+ {}", c.value());
                let val = c.value().clone();
                let whitespace_only = regex_space_only.is_match(&c.value());

                if !allow_add_spaces_once {
                    // error!("!allow_add_spaces_once");
                    return (false, "".to_string());
                }
                if whitespace_only {
                    allow_add_spaces_once = false;
                }
                if added_one_block {
                    // error!("added is more then one block!");
                    return (false, "".to_string());
                }
                if !deletion_once.is_empty() && !val.starts_with(&deletion_once.clone()) {
                    // error!("!deletion_once.is_empty() && !val.starts_with(&deletion_once.clone())");
                    return (false, "".to_string());
                }

                if adding_one_block && !is_multiline {
                    if !whitespace_only {
                        // error!("adding_one_block && !is_multiline && !whitespace_only");
                        return (false, "".to_string());
                    }
                }

                if deletion_once.is_empty() {
                    added_text += val;
                } else {
                    added_text += &val[deletion_once.len()..];
                }
                adding_one_block = true;
            }
            ChangeTag::Equal => {
                // info!("= {}", c.value());
                if adding_one_block {
                    added_one_block = true;
                }
            }
        }
    }
    if kill_slash_n {
        if added_text.ends_with("\n") {
            added_text = added_text[..added_text.len() - 1].to_string();
        }
    }
    added_text = added_text.replace("\r", "");
    (true, added_text)
}

pub fn unchanged_percentage(
    text_a: &String,
    text_b: &String,
) -> f64 {
    let diff = TextDiff::from_chars(text_a, text_b);
    let mut common = 0;
    for c in diff.iter_all_changes() {
        match c.tag() {
            ChangeTag::Delete => {
            }
            ChangeTag::Insert => {
            }
            ChangeTag::Equal => {
                common += c.value().len();
            }
        }
    }
    let largest_of_two = text_a.len().max(text_b.len());
    (common as f64) / (largest_of_two as f64)
}

fn common_characters_in_strings(a: &String, b: &String) -> i32 {
    let diff = TextDiff::from_chars(a, b);
    let mut common = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Delete => {}
            ChangeTag::Insert => {}
            ChangeTag::Equal => {
                common += 1;
            }
        }
    }
    common
}

pub fn unchanged_percentage_approx(
    text_a: &String,
    text_b: &String,
    grey_text_a: &String,
) -> f64 {
    struct BiggestCommon {
        val: i32,
        idx: usize,
        string: String,
        valid: bool,
    }

    trait BiggestCommonMethods {
        fn new() -> Self;
        fn compare(&mut self, new_val: i32, new_idx: usize, new_string: &String);
    }

    impl BiggestCommonMethods for BiggestCommon {
        fn new() -> Self {
            Self {
                val: 0,
                idx: 0,
                string: "".to_string(),
                valid: false,
            }
        }
        fn compare(&mut self, new_val: i32, new_idx: usize, new_string: &String) {
            if new_val > self.val {
                self.val = new_val;
                self.idx = new_idx;
                self.string = new_string.clone();
                self.valid = true;
            }
        }
    }

    let (texts_ab_added, _) = get_add_del_from_texts(text_a, text_b);

    // info!("unchanged_percentage_approx for snip:\n{grey_text_a}");
    if texts_ab_added.is_empty() {
        // info!("texts_ab_added.is_empty()");
        return 0.;
    }

    let mut common: i32 = 0;
    let mut a_idx_taken = vec![];
    for line in grey_text_a.lines() {
        // info!("checking line:\n{line}");

        let mut biggest_common = BiggestCommon::new();
        for (a_idx, a_line) in texts_ab_added.lines().enumerate() {
            if a_idx_taken.contains(&a_idx) {
                continue;
            }
            let a_common = common_characters_in_strings(&a_line.to_string(), &line.to_string());
            biggest_common.compare(a_common, a_idx, &a_line.to_string());
        }
        if !biggest_common.valid {
            continue;
        }
        // info!("most similar line: {}", biggest_common.string);
        // info!("biggest common: +{}/{}", biggest_common.val, line.len());
        a_idx_taken.push(biggest_common.idx);
        common += biggest_common.val;
    }
    common as f64 / grey_text_a.replace("\n", "").replace("\r", "").len() as f64
}
