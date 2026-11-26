//! Reduce confirmed native TaskCreate/TaskUpdate calls for the local task list.
//! No result text leaves this module.
use super::*;

enum Call {
    Create(String),
    Update { id: String, title: Option<String>, status: Option<String> },
}

#[derive(Default)]
pub(super) struct Tasks {
    pending: HashMap<String, Call>,
    tasks: Vec<(String, Task)>,
}

impl Tasks {
    pub fn call(&mut self, tool: &str, name: &str, input: &Value) {
        let call = match name {
            "TaskCreate" => string(&input["subject"]).map(Call::Create),
            "TaskUpdate" => string(&input["taskId"]).map(|id| Call::Update {
                id,
                title: string(&input["subject"]),
                status: string(&input["status"]),
            }),
            _ => None,
        };
        if let Some(call) = call {
            self.pending.insert(tool.into(), call);
        }
    }

    pub fn results(&mut self, content: &Value, structured: &Value) -> bool {
        let mut changed = false;
        let results = || blocks(content).filter(|block| block["type"] == "tool_result");
        let single_result = results().take(2).count() == 1;
        for block in results() {
            let Some(id) = block["tool_use_id"].as_str() else { continue };
            let Some(call) = self.pending.remove(id) else { continue };
            if flag(block, "is_error") || (single_result && structured["success"] == false) {
                continue;
            }
            match call {
                Call::Create(content) => {
                    let result = &block["content"];
                    // The structured response belongs to the whole record, so don't use it for
                    // a record carrying more than one result.
                    let id = single_result
                        .then(|| value_id(&structured["task"]["id"]))
                        .flatten()
                        .or_else(|| text_id(result));
                    if let Some(id) = id
                        && self.tasks.iter().all(|(key, _)| key != &id)
                    {
                        self.tasks.push((id, Task { content, status: TaskStatus::Pending }));
                        changed = true;
                    }
                }
                Call::Update { id, title, status } => {
                    if let Some(index) = self.tasks.iter().position(|(key, _)| key == &id) {
                        if status.as_deref() == Some("deleted") {
                            self.tasks.remove(index);
                        } else {
                            let task = &mut self.tasks[index].1;
                            if let Some(title) = title {
                                task.content = title;
                            }
                            task.status = match status.as_deref() {
                                Some("pending") => TaskStatus::Pending,
                                Some("in_progress") => TaskStatus::InProgress,
                                Some("completed") => TaskStatus::Completed,
                                _ => task.status,
                            };
                        }
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    pub fn finish(self) -> Vec<Task> {
        self.tasks.into_iter().map(|(_, task)| task).collect()
    }
}

fn value_id(value: &Value) -> Option<String> {
    value.as_str().filter(|id| !id.is_empty()).map(str::to_owned).or_else(|| value.as_u64().map(|id| id.to_string()))
}

fn text_id(value: &Value) -> Option<String> {
    let text = value
        .as_str()
        .or_else(|| blocks(value).find_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten()))?;
    let (id, _) = text.strip_prefix("Task #")?.split_once(" created successfully")?;
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then(|| id.to_owned())
}
