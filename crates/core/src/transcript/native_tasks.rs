//! Reduce confirmed native TaskCreate/TaskUpdate calls for the local task list.
//! No result text leaves this module.
use super::*;

#[derive(Clone, Debug)]
pub(super) enum Call {
    Create(String),
    Update { id: String, title: Option<String>, status: Option<String> },
}

impl Call {
    pub fn parse(name: &str, input: &Value) -> Option<Self> {
        match name {
            "TaskCreate" => string(&input["subject"]).map(Call::Create),
            "TaskUpdate" => string(&input["taskId"]).map(|id| Call::Update {
                id,
                title: string(&input["subject"]),
                status: string(&input["status"]),
            }),
            _ => None,
        }
    }
}

/// How one tool result ended: whether it succeeded and the task id a creation reported.
#[derive(Clone, Debug)]
pub(super) struct Outcome {
    pub tool: String,
    ok: bool,
    created: Option<String>,
}

/// The outcomes of a user record's tool results.
pub(super) fn outcomes(content: &Value, structured: &Value) -> Vec<Outcome> {
    let results = || blocks(content).filter(|block| block["type"] == "tool_result");
    let single_result = results().take(2).count() == 1;
    results()
        .filter_map(|block| {
            let tool = block["tool_use_id"].as_str()?.to_owned();
            let ok = !flag(block, "is_error") && !(single_result && structured["success"] == false);
            // The structured response belongs to the whole record, so don't use it for a record
            // carrying more than one result.
            let created = single_result
                .then(|| value_id(&structured["task"]["id"]))
                .flatten()
                .or_else(|| text_id(&block["content"]));
            Some(Outcome { tool, ok, created })
        })
        .collect()
}

#[derive(Default)]
pub(super) struct Tasks {
    pending: HashMap<String, Call>,
    tasks: Vec<(String, Task)>,
}

impl Tasks {
    pub fn call(&mut self, tool: &str, name: &str, input: &Value) {
        if let Some(call) = Call::parse(name, input) {
            self.pending.insert(tool.into(), call);
        }
    }

    pub fn pend(&mut self, tool: &str, call: Call) {
        self.pending.insert(tool.into(), call);
    }

    pub fn results(&mut self, content: &Value, structured: &Value) -> bool {
        let mut changed = false;
        for outcome in outcomes(content, structured) {
            changed |= self.outcome(outcome, false);
        }
        changed
    }

    /// Applies a result to the call it answers, `rewound` when that call is in a turn a rewind
    /// replaced (the task's state then comes from there); true when the list changed.
    pub fn outcome(&mut self, outcome: Outcome, rewound: bool) -> bool {
        let Some(call) = self.pending.remove(&outcome.tool) else { return false };
        if !outcome.ok {
            return false;
        }
        match call {
            Call::Create(content) => {
                if let Some(id) = outcome.created
                    && self.tasks.iter().all(|(key, _)| key != &id)
                {
                    self.tasks.push((id, Task { content, status: TaskStatus::Pending, rewound }));
                    return true;
                }
                false
            }
            Call::Update { id, title, status } => {
                let Some(index) = self.tasks.iter().position(|(key, _)| key == &id) else { return false };
                if status.as_deref() == Some("deleted") {
                    self.tasks.remove(index);
                } else {
                    let task = &mut self.tasks[index].1;
                    task.rewound = rewound;
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
                true
            }
        }
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
