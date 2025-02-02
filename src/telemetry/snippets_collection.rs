use std::sync::Arc;
use std::sync::RwLock as StdRwLock;
use serde::{Serialize, Deserialize};

use tokio::sync::RwLock as ARwLock;

use crate::global_context;
use crate::completion_cache;
use crate::call_validation::CodeCompletionPost;
use crate::telemetry::telemetry_structs;
use crate::telemetry::basic_robot_human;
use crate::telemetry::basic_comp_counters;
use crate::telemetry::utils;


// How it works:
// 1. Rust returns {"snippet_telemetry_id":101,"choices":[{"code_completion":"\n    return \"Hello World!\"\n"}] ...}
// 2. IDE detects accept, sends /v1/completion-accepted with {"snippet_telemetry_id":101}
// 3. LSP looks at file changes
// 4. Changes are translated to base telemetry counters


#[derive(Debug, Clone)]
pub struct SaveSnippet {
    // Purpose is to aggregate this struct to a scratchpad
    pub storage_arc: Arc<StdRwLock<telemetry_structs::Storage>>,
    pub post: CodeCompletionPost,
}

impl SaveSnippet {
    pub fn new(
        storage_arc: Arc<StdRwLock<telemetry_structs::Storage>>,
        post: &CodeCompletionPost
    ) -> Self {
        SaveSnippet {
            storage_arc,
            post: post.clone(),
        }
    }
}

fn snippet_register(
    ss: &SaveSnippet,
    grey_text: String,
) -> u64 {
    let mut storage_locked = ss.storage_arc.write().unwrap();
    let snippet_telemetry_id = storage_locked.tele_snippet_next_id;
    let snip = telemetry_structs::SnippetTracker {
        snippet_telemetry_id,
        model: ss.post.model.clone(),
        inputs: ss.post.inputs.clone(),
        grey_text: grey_text.clone(),
        corrected_by_user: "".to_string(),
        remaining_percentage: -1.,
        created_ts: chrono::Local::now().timestamp(),
        accepted_ts: 0,
        finished_ts: 0,
    };
    storage_locked.tele_snippet_next_id += 1;
    storage_locked.tele_snippets.push(snip);
    snippet_telemetry_id
}

pub fn snippet_register_from_data4cache(
    ss: &SaveSnippet,
    data4cache: &mut completion_cache::CompletionSaveToCache,
) {
    // Convenience function: snippet_telemetry_id should be returned inside a cached answer as well, so there's
    // typically a combination of the two
    if data4cache.completion0_finish_reason.is_empty() {
        return;
    }
    data4cache.completion0_snippet_telemetry_id = Some(snippet_register(&ss, data4cache.completion0_text.clone()));
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SnippetAccepted {
    pub snippet_telemetry_id: u64,
}

pub async fn snippet_accepted(
    gcx: Arc<ARwLock<global_context::GlobalContext>>,
    snippet_telemetry_id: u64,
) -> bool {
    let tele_storage_arc = gcx.read().await.telemetry.clone();
    let mut storage_locked = tele_storage_arc.write().unwrap();
    let snip = storage_locked.tele_snippets.iter_mut().find(|s| s.snippet_telemetry_id == snippet_telemetry_id);
    if let Some(snip) = snip {
        snip.accepted_ts = chrono::Local::now().timestamp();
        return true;
    }
    return false;
}


pub async fn sources_changed(
    gcx: Arc<ARwLock<global_context::GlobalContext>>,
    uri: &String,
    text: &String,
) {
    let tele_storage = gcx.read().await.telemetry.clone();
    let mut storage_locked = tele_storage.write().unwrap();
    let mut finished_snips = vec![];
    for snip in storage_locked.tele_snippets.iter_mut() {
        if snip.accepted_ts == 0 || !uri.ends_with(&snip.inputs.cursor.file) {
            continue;
        }
        if snip.finished_ts > 0 {
            continue;
        }
        let orig_text = snip.inputs.sources.get(&snip.inputs.cursor.file);
        if !orig_text.is_some() {
            continue;
        }
        let (grey_valid, mut grey_corrected) = utils::if_head_tail_equal_return_added_text(
            orig_text.unwrap(),
            text,
            &snip.grey_text,
        );
        if grey_valid {
            let unchanged_percentage = utils::unchanged_percentage(&grey_corrected, &snip.grey_text);
            grey_corrected = grey_corrected.replace("\r", "");
            snip.corrected_by_user = grey_corrected.clone();
            snip.remaining_percentage = unchanged_percentage;
        } else {
            if snip.remaining_percentage >= 0. {
                snip.finished_ts = chrono::Local::now().timestamp();
                // info!("snip {} is finished with score={}!", snip.grey_text, snip.remaining_percentage);
                finished_snips.push(snip.clone());
            } else {
                // info!("snip {} is finished with accepted = false", snip.grey_text);
                snip.accepted_ts = 0;  // that will cleanup and not send
            }
        }
    }

    for snip in finished_snips {
        basic_robot_human::increase_counters_from_finished_snippet(&mut storage_locked.tele_robot_human, uri, text, &snip);
        basic_comp_counters::create_data_accumulator_for_finished_snippet(&mut storage_locked.snippet_data_accumulators, uri, &snip);
    }
    basic_comp_counters::on_file_text_changed(&mut storage_locked.snippet_data_accumulators, uri, text);
}
