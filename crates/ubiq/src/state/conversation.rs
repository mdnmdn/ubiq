//! One live agent's conversation, as the window holds it.
//!
//! The host is the only writer. What arrives is a stream of deltas — a chunk
//! appends to the block it belongs to, a tool-call patch changes only the
//! fields it names — and this is where they are folded into something a
//! transcript can draw. Nothing here invents a line: the composer appends
//! nothing when it sends, and the user's own turn appears when the harness
//! echoes it back.
//!
//! What *is* derived here is presentation: the run pill, the activity badge,
//! the context ring. Those are read off the stream rather than asked for,
//! because a second round trip per token would be a round trip per token.

use std::collections::{BTreeMap, HashMap};

use ubiq_proto::conversation::{
    ConfigOption, ConfigValue, ConvContent, ConvUpdate, PermissionOption, PlanEntry,
    RateLimitRecord, StopReason, Subagent, TokenSpend, ToolCallPatch, ToolCallRecord, ToolStatus,
    UsageRecord,
};
use ubiq_proto::work::{Activity, AgentId};

/// One thing in a transcript, in the order it was said.
///
/// **Three of the four carry who said it.** A spawned subagent's prose, reasoning and tool calls
/// all arrive on the same stream as the conversation's own, and a block that lost the attribution
/// would be drawn as the main agent's own voice — which is exactly the transcript telling a lie
/// about who spoke.
#[derive(Clone, Debug, PartialEq)]
pub enum ConvBlock {
    /// What the user said, as the harness received it.
    User(String),
    /// Assistant prose, markdown. `subagent` is `None` for the conversation itself.
    Agent {
        body: String,
        subagent: Option<Subagent>,
    },
    /// Reasoning.
    Thought {
        body: String,
        subagent: Option<Subagent>,
    },
    /// A tool call and whether its detail is open.
    Tool { call: ToolCallRecord, open: bool },
}

impl ConvBlock {
    /// Which subagent produced this block, where one did.
    pub fn subagent(&self) -> Option<&Subagent> {
        match self {
            ConvBlock::User(_) => None,
            ConvBlock::Agent { subagent, .. } | ConvBlock::Thought { subagent, .. } => {
                subagent.as_ref()
            }
            ConvBlock::Tool { call, .. } => call.subagent.as_ref(),
        }
    }

    /// Which *instance* produced it — the only identity a transcript can be filtered by, since
    /// several subagents of one type are one type and several agents.
    pub fn subagent_id(&self) -> Option<&str> {
        self.subagent().map(|who| who.id.as_str())
    }
}

/// One spawned subagent, as the switcher above the composer draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct SubagentTab {
    /// The spawning tool call's id, which is what [`Conversation::viewing`] holds.
    pub id: String,
    pub name: String,
    /// What it is doing, read off the spawning call rather than invented here. `None` where that
    /// call is not in the transcript at all: nothing here knows what such an agent is up to, and
    /// saying "running" would be a guess drawn as a fact.
    pub status: Option<ToolStatus>,
    /// The type the harness named — `"general-purpose"` — where it named one.
    pub kind: Option<String>,
    /// What this delegate is answering with, as the harness stamped its lines. Its own model, not
    /// the parent's: a delegate launched on a smaller model is exactly what a reader is looking
    /// for here.
    pub model: Option<String>,
    /// What effort it runs at, where the harness says. `None` on every harness today — nothing in
    /// a stream states a per-delegate level — and drawn as nothing rather than borrowed from the
    /// parent conversation, which would be a guess wearing a fact's clothes.
    pub thinking: Option<String>,
}

/// Fold one report's spend into a running total. Field by field, because a flow is summed and
/// there is no other way to sum one; saturating, because a counter that wrapped would read as a
/// conversation that spent nothing.
fn accumulate(into: &mut TokenSpend, add: &TokenSpend) {
    into.input = into.input.saturating_add(add.input);
    into.output = into.output.saturating_add(add.output);
    into.thinking = into.thinking.saturating_add(add.thinking);
    into.cache_read = into.cache_read.saturating_add(add.cache_read);
    into.cache_creation = into.cache_creation.saturating_add(add.cache_creation);
}

/// A permission the agent is waiting on.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub request_id: String,
    pub tool_call: ToolCallPatch,
    pub options: Vec<PermissionOption>,
}

/// A prompt typed while a turn was already running, held until it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedMessage {
    pub id: u64,
    pub text: String,
}

/// Whether the agent is working or waiting for a turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Idle,
    Working,
    Ended,
}

impl Run {
    pub fn label(self) -> &'static str {
        match self {
            Run::Idle => "Idle",
            Run::Working => "Working",
            Run::Ended => "Ended",
        }
    }
}

/// A live agent's conversation.
#[derive(Clone, Debug)]
pub struct Conversation {
    pub id: AgentId,
    /// The harness's display name, from the record the host minted.
    pub harness: String,
    /// The identity it runs as, empty when it resolved none and fell back to the user's own
    /// home. Fixed for the conversation's life: a turn already taken was taken as somebody.
    pub account: String,
    pub blocks: Vec<ConvBlock>,
    /// What the harness says it is answering with. Empty until it says.
    pub model: Option<String>,
    pub mode: Option<String>,
    /// The title the harness has given this conversation, where it names one. `None` until it
    /// does — `refresh_agent_record` in `app.rs` is what turns this into the name a reader
    /// actually sees (the sidebar row, the column header, the chat panel row).
    pub title: Option<String>,
    /// Context and cost, as of the last thing the harness reported **for the conversation itself**.
    /// Occupancy is a level: it is replaced, never summed, and a subagent's report never reaches
    /// it — a subagent repeats the parent's `used`/`size` unchanged, so applying one would move the
    /// ring for a turn that did not touch the window.
    pub usage: Option<UsageRecord>,
    /// Every token this conversation has billed, folded as reports arrive. A flow, so it is summed
    /// — the opposite of [`Self::usage`]. `None` until the harness counts anything, which is what
    /// lets the footer draw nothing rather than a zero it made up.
    pub spend: Option<TokenSpend>,
    /// The same total, split by who spent it: the key is the subagent type, and the empty string is
    /// the conversation's own turns. What answers "who burned the tokens" without leaving the chat.
    pub spend_by_subagent: BTreeMap<String, TokenSpend>,
    /// How full the user's rate-limit windows are, as of the last thing the harness reported.
    pub rate_limit: Option<RateLimitRecord>,
    pub run: Run,
    pub stop_reason: Option<StopReason>,
    /// What the harness advertised: the model, the mode, the thinking level.
    /// One list, because upstream has one mechanism for all of them.
    pub config: Vec<ConfigOption>,
    pub plan: Vec<PlanEntry>,
    pub pending: Option<Pending>,
    /// The last thing that went wrong, until the next thing happens.
    pub error: Option<String>,
    /// Whether this harness takes a second turn at all.
    pub accepts_input: bool,
    /// What the composer holds, unsent.
    pub draft: String,
    /// Whether the harness behind this conversation has actually launched. `false` from
    /// registration until its own `Started` event arrives — the window between them is P3's
    /// pending stage, where the composer offers a model picker instead of a running conversation.
    pub launched: bool,
    /// What's been picked before launch, optimistically, keyed by `config_id` — the host does not
    /// echo a `SetAgentConfig` sent while a conversation is still pending, so this is what each
    /// picker highlights until `launched` flips [`Self::launched`] true. An id with no entry means
    /// "the harness's own default", which is also that `ConfigOption`'s own `current` field until
    /// the user picks something else. A map rather than one field per knob: the host can grow a
    /// fourth config id without a change here.
    pub chosen: BTreeMap<String, String>,
    /// Which of the pre-launch pickers (by `config_id`) is open, if any. Kept on the conversation
    /// rather than the window's single `open_menu` — several conversations can be on screen
    /// pending at once, each with its own pickers — but still one at a time per conversation, the
    /// same rule the window's menus follow.
    pub open_config: Option<String>,
    /// Which spawned subagent's transcript is being read — its instance id — `None` being the
    /// conversation's own turns. Beside [`Self::open_config`] and for its reason: several
    /// conversations are on screen at once and each is read independently, so this cannot live on
    /// the window. Read through [`Self::viewing_subagent`], which discounts an id the transcript
    /// no longer has.
    pub viewing: Option<String>,
    /// Whether the subagent panel is open. Collapsed by default and per conversation, beside
    /// [`Self::viewing`] and for its reason: several conversations are on screen at once, and each
    /// reader opens the ones they are following.
    pub subagents_open: bool,
    /// Prompts typed while a turn was already running, held until it ends. A stable
    /// per-conversation id per entry, so an edit or a delete names the right one even if others
    /// are added or removed around it.
    pub queued: Vec<QueuedMessage>,
    next_queued_id: u64,

    /// The highest sequence number applied. An update that does not follow it
    /// is a gap, and a gap is worth saying rather than silently drawing.
    seq: u64,
    /// Which block each tool call is, so a patch reaches it in one lookup
    /// rather than a scan of the whole transcript.
    tools: HashMap<String, usize>,
    /// The message currently being appended to, and which block it is. A
    /// change of id starts a new block — that is what a message id is for.
    open: Option<(String, usize)>,
}

impl Conversation {
    pub fn new(id: AgentId, harness: String, account: String) -> Self {
        Self {
            id,
            harness,
            account,
            blocks: Vec::new(),
            model: None,
            mode: None,
            title: None,
            usage: None,
            spend: None,
            spend_by_subagent: BTreeMap::new(),
            rate_limit: None,
            run: Run::Idle,
            stop_reason: None,
            config: Vec::new(),
            plan: Vec::new(),
            pending: None,
            error: None,
            accepts_input: true,
            draft: String::new(),
            launched: false,
            chosen: BTreeMap::new(),
            open_config: None,
            viewing: None,
            subagents_open: false,
            queued: Vec::new(),
            next_queued_id: 0,
            seq: 0,
            tools: HashMap::new(),
            open: None,
        }
    }

    /// What a subagent is called: the title of the `Task` call that spawned it — the bridge
    /// titles that with the task's own description, "Formal greeting agent" — falling back to its
    /// type when no such call is in the transcript, and to the raw id when the harness named
    /// neither.
    ///
    /// The one place the join is done. Two callers resolving it apart would eventually disagree
    /// about what the same agent is called.
    pub fn subagent_name(&self, id: &str) -> String {
        if let Some(ConvBlock::Tool { call, .. }) = self
            .blocks
            .iter()
            .find(|block| matches!(block, ConvBlock::Tool { call, .. } if call.id == id))
        {
            return call.title.clone();
        }
        self.subagent_stamps(id)
            .find_map(|who| who.kind.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// Every subagent this conversation has spawned, in the order it first spoke — one tag each in
    /// the switcher. Distinct by instance, so three `general-purpose` agents are three tags.
    ///
    /// Status comes from the spawning call, which is the same `ToolStatus` the transcript already
    /// draws for it; a subagent whose spawning call is not in the transcript has no status at all,
    /// and the row says so rather than claiming one.
    pub fn subagents(&self) -> Vec<SubagentTab> {
        let mut tabs: Vec<SubagentTab> = Vec::new();
        for id in self.blocks.iter().filter_map(ConvBlock::subagent_id) {
            if tabs.iter().any(|tab| tab.id == id) {
                continue;
            }
            tabs.push(SubagentTab {
                id: id.to_string(),
                name: self.subagent_name(id),
                status: self.subagent_status(id),
                kind: self.subagent_stamps(id).find_map(|who| who.kind.clone()),
                model: self.subagent_stamps(id).find_map(|who| who.model.clone()),
                thinking: self
                    .subagent_stamps(id)
                    .find_map(|who| who.thinking.clone()),
            });
        }
        tabs
    }

    /// Every stamp this delegate's own lines carry, in order — its kind, and what it was launched
    /// to answer with.
    ///
    /// Each field is read from the first line that *names* it rather than from the first line
    /// outright: the harness resolves a delegate's model once, at launch, and repeats it on every
    /// line of that delegate, so a line that names none is simply not the one to read it from.
    fn subagent_stamps(&self, id: &str) -> impl Iterator<Item = &Subagent> {
        self.blocks
            .iter()
            .filter_map(ConvBlock::subagent)
            .filter(move |who| who.id == id)
    }

    fn subagent_status(&self, id: &str) -> Option<ToolStatus> {
        self.blocks.iter().find_map(|block| match block {
            ConvBlock::Tool { call, .. } if call.id == id => Some(call.status),
            _ => None,
        })
    }

    /// Whether anything in the transcript was said by this subagent — which is the only
    /// evidence the window has that the agent exists at all. What makes a delegation block a way
    /// in to a second transcript rather than a dead label.
    pub fn has_subagent(&self, id: &str) -> bool {
        self.blocks
            .iter()
            .any(|block| block.subagent_id() == Some(id))
    }

    /// Whose transcript is on screen, once a stale id is discounted: a subagent that has gone from
    /// the transcript — a resume, a cleared history — falls back to the main agent rather than
    /// leaving the reader looking at nothing.
    pub fn viewing_subagent(&self) -> Option<&str> {
        let id = self.viewing.as_deref()?;
        self.has_subagent(id).then_some(id)
    }

    /// The blocks to draw, for whoever is being read — with their real indices, because an
    /// element id and the tool-toggle listener both key off a block's position in `blocks`.
    ///
    /// One transcript at a time: the main agent's own turns exclude every subagent's, and a
    /// subagent's include only its own. The rule lives here so the switcher and the transcript
    /// cannot disagree about it.
    pub fn visible_blocks(&self) -> Vec<(usize, &ConvBlock)> {
        let viewing = self.viewing_subagent();
        self.blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.subagent_id() == viewing)
            .collect()
    }

    /// The badge the sidebar and the column header draw.
    ///
    /// Derived rather than carried: an activity is a reading of the last
    /// thing that happened, and the stream already says what that was.
    pub fn activity(&self) -> Activity {
        if self.pending.is_some() {
            return Activity::NeedsYou;
        }
        match (self.run, self.stop_reason) {
            (Run::Ended, Some(StopReason::Failed)) => Activity::Failed,
            (Run::Ended, _) => Activity::Ended,
            // Never run a turn: either a pending conversation whose harness has not launched yet,
            // or one that just has — read as still getting itself going, matching the
            // `Activity::Thinking` the host reports at registration, rather than as ended before
            // it began.
            (Run::Idle, None) => Activity::Thinking,
            (Run::Idle, Some(_)) => Activity::Ended,
            (Run::Working, _) => match self.blocks.last() {
                Some(ConvBlock::Thought { .. }) => Activity::Thinking,
                Some(ConvBlock::Tool { .. }) => Activity::Tools,
                _ => Activity::Writing,
            },
        }
    }

    /// The percentage the ring draws, when a window is known.
    pub fn context_pct(&self) -> Option<u8> {
        self.usage.as_ref().and_then(UsageRecord::context_pct)
    }

    /// Tokens in the context, as a count rather than a ratio.
    pub fn tokens(&self) -> u64 {
        self.usage.as_ref().map_or(0, |usage| usage.used)
    }

    /// What the turn cost so far, where the harness reports money.
    pub fn cost_usd(&self) -> Option<f64> {
        self.usage.as_ref().and_then(|usage| usage.cost_usd)
    }

    /// Every token this conversation has spent, its subagents included, accumulated over every
    /// report rather than read off the last one. Not the ring: [`Self::tokens`] is what still
    /// occupies the context window, and this is what has gone through it. `None` means the harness
    /// counted nothing, which the footer draws as nothing rather than as a zero it made up.
    pub fn total_tokens(&self) -> Option<u64> {
        self.spend.map(|spend| spend.total())
    }

    /// The context-re-use part of that total, where it is reported. Cache *creation* is not in it:
    /// that is context paid for once, not context saved.
    pub fn cached_tokens(&self) -> Option<u64> {
        self.spend.map(|spend| spend.cached())
    }

    /// What each spawned subagent type spent, biggest first, and never the conversation's own
    /// entry — this is the breakdown beside the total, not a second copy of it.
    pub fn subagent_spend(&self) -> Vec<(&str, u64)> {
        let mut rows: Vec<(&str, u64)> = self
            .spend_by_subagent
            .iter()
            .filter(|(name, spend)| !name.is_empty() && spend.total() > 0)
            .map(|(name, spend)| (name.as_str(), spend.total()))
            .collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        rows
    }

    /// How full the rolling five-hour rate-limit window is, where the harness reports one.
    pub fn rate_limit_five_hour_pct(&self) -> Option<u8> {
        self.rate_limit.as_ref().and_then(|r| r.five_hour_pct)
    }

    /// Whether a `seq` follows the last one applied.
    ///
    /// The bus promises order per agent, so a gap means a message was lost
    /// rather than reordered — worth reporting, and worth applying anyway,
    /// since half a transcript beats none.
    pub fn is_next(&self, seq: u64) -> bool {
        seq == self.seq + 1
    }

    /// Fold one delta in.
    pub fn apply(&mut self, seq: u64, update: ConvUpdate) {
        self.seq = self.seq.max(seq);
        self.error = None;

        match update {
            ConvUpdate::Started { model, mode, .. } => {
                self.model = model;
                self.mode = mode;
                self.launched = true;
            }

            ConvUpdate::UserChunk { content, .. } => {
                // A user turn always starts a block: the harness echoes one
                // per prompt, and merging two would merge two questions.
                self.open = None;
                if let Some(text) = text_of(&content) {
                    self.blocks.push(ConvBlock::User(text));
                }
                self.run = Run::Working;
            }
            ConvUpdate::AgentChunk {
                content,
                message_id,
                subagent,
            } => {
                self.run = Run::Working;
                self.append(message_id, content, false, subagent);
            }
            ConvUpdate::ThoughtChunk {
                content,
                message_id,
                subagent,
            } => {
                self.run = Run::Working;
                self.append(message_id, content, true, subagent);
            }

            ConvUpdate::ToolCall(call) => {
                self.run = Run::Working;
                self.open = None;
                self.tools.insert(call.id.clone(), self.blocks.len());
                self.blocks.push(ConvBlock::Tool { call, open: false });
            }
            ConvUpdate::ToolCallUpdate(patch) => self.patch_tool(patch),

            ConvUpdate::Plan(entries) => self.plan = entries,
            ConvUpdate::ConfigOptions(options) => {
                self.config = options;
                // A model pick makes the host re-send `ConfigOptions` with e.g. the thinking
                // levels recomputed for the newly chosen model — a level the old model accepted
                // may not exist under the new one. Drop any held pick the fresh options no longer
                // back, and drop picks for ids that vanished entirely, so a stale choice never
                // survives into launch.
                let config = &self.config;
                self.chosen.retain(|config_id, value| {
                    config
                        .iter()
                        .find(|opt| &opt.id == config_id)
                        .is_some_and(|opt| match &opt.value {
                            ConfigValue::Select { choices, .. } => {
                                choices.iter().any(|choice| &choice.value == value)
                            }
                            ConfigValue::Flag { .. } => true,
                        })
                });
            }
            ConvUpdate::ModeChanged { mode_id } => self.mode = Some(mode_id),
            // Held here; `refresh_agent_record` (`app.rs`) is what copies it onto the
            // `WorkAgent` the sidebar, the column header and the chat panel actually read.
            ConvUpdate::Title(title) => self.title = Some(title),

            ConvUpdate::Usage(usage) => {
                // Spend is a flow, and every report's flow counts — the conversation's own and
                // each subagent's, kept apart as well as together.
                if let Some(spend) = usage.spend {
                    accumulate(self.spend.get_or_insert_default(), &spend);
                    accumulate(
                        self.spend_by_subagent
                            .entry(usage.subagent.clone().unwrap_or_default())
                            .or_default(),
                        &spend,
                    );
                }
                // A subagent's report repeats the parent's occupancy and names the subagent's own
                // model. Neither is news about this conversation: it is a spend row, and it stops
                // here rather than moving the ring or renaming the column.
                if usage.subagent.is_some() {
                    return;
                }
                // A model is only named where the harness named it: a usage
                // report for a fallback model must not rename the column.
                if usage.model.is_some() {
                    self.model = usage.model.clone();
                }
                self.usage = Some(usage);
            }

            ConvUpdate::RateLimit(record) => self.rate_limit = Some(record),

            ConvUpdate::PermissionRequest {
                request_id,
                tool_call,
                options,
            } => {
                self.pending = Some(Pending {
                    request_id,
                    tool_call,
                    options,
                });
            }

            ConvUpdate::TurnEnded { stop_reason, error } => {
                self.open = None;
                self.run = Run::Idle;
                self.stop_reason = Some(stop_reason);
                self.error = error;
            }
        }
    }

    /// The harness has gone.
    pub fn ended(&mut self, stop_reason: StopReason) {
        self.open = None;
        self.pending = None;
        self.run = Run::Ended;
        self.stop_reason = Some(stop_reason);
    }

    /// The harness is gone but the conversation is not: back to the state it had before its first
    /// turn, so the pickers return and the next prompt — or a resume — starts a new process.
    /// `blocks` stays exactly as it is; only a resumed harness's own transcript will grow it
    /// further.
    pub fn unloaded(&mut self) {
        self.run = Run::Idle;
        self.launched = false;
        self.open_config = None;
        self.pending = None;
    }

    /// Hold a prompt for later, typed while a turn was already running. Returns the id it was
    /// given, so a caller can find this entry again to edit or delete it.
    pub fn enqueue(&mut self, text: String) -> u64 {
        let id = self.next_queued_id;
        self.next_queued_id += 1;
        self.queued.push(QueuedMessage { id, text });
        id
    }

    /// Pop the oldest queued prompt — what a turn ending sends automatically.
    pub fn dequeue_front(&mut self) -> Option<QueuedMessage> {
        (!self.queued.is_empty()).then(|| self.queued.remove(0))
    }

    /// Take a queued prompt's text back out, by id. A plain delete, or the first half of an edit
    /// — the caller loads what comes back into the live composer.
    pub fn remove_queued(&mut self, id: u64) -> Option<String> {
        let ix = self.queued.iter().position(|m| m.id == id)?;
        Some(self.queued.remove(ix).text)
    }

    /// Toggle a tool block's detail.
    pub fn toggle_tool(&mut self, id: &str) {
        if let Some(ConvBlock::Tool { open, .. }) =
            self.tools.get(id).map(|ix| &mut self.blocks[*ix])
        {
            *open = !*open;
        }
    }

    /// Append a chunk to the message it belongs to, starting a new block when
    /// the message id changes — which is what a message id is for.
    ///
    /// A chunk with no id at all can only extend the block immediately before
    /// it, and only if that block is the same kind: a harness that numbers
    /// nothing still streams in order.
    ///
    /// **Who said it is part of what makes two chunks one block.** A subagent's message id is
    /// minted by its own turn and can collide with the parent's, so matching on id and kind alone
    /// lands a spawned agent's greeting inside the sentence the main agent was in the middle of.
    fn append(
        &mut self,
        message_id: Option<String>,
        content: ConvContent,
        thought: bool,
        subagent: Option<Subagent>,
    ) {
        let Some(text) = text_of(&content) else {
            return;
        };

        let same_block = |block: &ConvBlock| {
            matches!(
                (block, thought),
                (ConvBlock::Agent { .. }, false) | (ConvBlock::Thought { .. }, true)
            ) && block.subagent_id() == subagent.as_ref().map(|who| who.id.as_str())
        };

        if let Some((open_id, ix)) = &self.open
            && message_id.as_ref().is_none_or(|id| id == open_id)
            && self.blocks.get(*ix).is_some_and(same_block)
        {
            match &mut self.blocks[*ix] {
                ConvBlock::Agent { body, .. } | ConvBlock::Thought { body, .. } => {
                    body.push_str(&text)
                }
                _ => unreachable!("same_block just matched one of these two"),
            }
            return;
        }

        let ix = self.blocks.len();
        self.blocks.push(if thought {
            ConvBlock::Thought {
                body: text,
                subagent,
            }
        } else {
            ConvBlock::Agent {
                body: text,
                subagent,
            }
        });
        self.open = Some((message_id.unwrap_or_default(), ix));
    }

    /// Apply a tool-call patch: **absent means unchanged**, and content and
    /// locations replace rather than append.
    fn patch_tool(&mut self, patch: ToolCallPatch) {
        let Some(ix) = self.tools.get(&patch.id).copied() else {
            // A result for a call nobody announced. Dropping it is right:
            // drawing a completed call with no beginning would invent one.
            return;
        };
        let Some(ConvBlock::Tool { call, .. }) = self.blocks.get_mut(ix) else {
            return;
        };

        if let Some(title) = patch.title {
            call.title = title;
        }
        if let Some(kind) = patch.kind {
            call.kind = kind;
        }
        if let Some(status) = patch.status {
            call.status = status;
        }
        if let Some(content) = patch.content {
            call.content = content;
        }
        if let Some(locations) = patch.locations {
            call.locations = locations;
        }
    }
}

/// The short name the composer's model chip wears, given the harness that answered.
///
/// Only Claude's ids are shortened. They read `vendor-family-version-date`, and the family is the
/// only part anyone picks a model by — `claude-haiku-4-5-20251001` is `haiku`. Every other harness
/// names its models however it likes, so its id is left exactly as it came: a cut guessed across
/// vendors would take the meaning out of half of them. The full id is never lost — the chip's
/// tooltip carries it.
pub fn short_model_label(harness: &str, model: &str) -> String {
    if !harness.to_lowercase().contains("claude") {
        return model.to_string();
    }
    model.split('-').nth(1).unwrap_or(model).to_string()
}

fn text_of(content: &ConvContent) -> Option<String> {
    match content {
        ConvContent::Text(text) => Some(text.clone()),
        // Something the transcript cannot draw. Saying it arrived beats
        // dropping it silently, and beats pretending it was prose.
        ConvContent::Other { kind, description } => Some(format!("_[{kind}: {description}]_")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::conversation::{TokenSpend, ToolKind, ToolStatus};

    fn task_call(id: &str, title: &str, status: ToolStatus) -> ConvUpdate {
        ConvUpdate::ToolCall(ToolCallRecord {
            id: id.to_string(),
            title: title.to_string(),
            kind: ToolKind::Other,
            status,
            content: Vec::new(),
            locations: Vec::new(),
            subagent: None,
        })
    }

    fn said_by(text: &str, id: &str, kind: Option<&str>) -> ConvUpdate {
        ConvUpdate::AgentChunk {
            content: ConvContent::Text(text.to_string()),
            message_id: Some("m1".to_string()),
            subagent: Some(Subagent {
                id: id.to_string(),
                kind: kind.map(str::to_string),
                ..Default::default()
            }),
        }
    }

    /// Turn 2 of `_data/ubiq-tape-1788688032.jsonl`: three `general-purpose` subagents, spawned by
    /// three `Task` calls. The type they share is not their identity — keying on it would collapse
    /// them into one tag with their turns interleaved, which is the bug the instance id fixes.
    fn three_greeters() -> Conversation {
        let mut c = conversation();
        c.apply(1, chunk("I'll delegate.", Some("m0")));
        c.apply(
            2,
            task_call("t751", "Formal greeting agent", ToolStatus::Completed),
        );
        c.apply(
            3,
            task_call("t754", "Pirate-style greeting agent", ToolStatus::Failed),
        );
        c.apply(
            4,
            task_call("t759", "Poetic greeting agent", ToolStatus::InProgress),
        );
        c.apply(5, said_by("Good day.", "t751", Some("general-purpose")));
        c.apply(6, said_by("Ahoy!", "t754", Some("general-purpose")));
        c.apply(
            7,
            said_by("A greeting, in verse.", "t759", Some("general-purpose")),
        );
        c
    }

    #[test]
    fn three_subagents_of_one_kind_stay_three_agents() {
        let c = three_greeters();
        let tabs = c.subagents();
        assert_eq!(
            tabs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            vec!["t751", "t754", "t759"],
            "one tag per instance, in the order each first spoke"
        );
        assert_eq!(
            tabs.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            vec![
                "Formal greeting agent",
                "Pirate-style greeting agent",
                "Poetic greeting agent"
            ],
        );
        assert_eq!(
            tabs.iter().map(|t| t.status).collect::<Vec<_>>(),
            vec![
                Some(ToolStatus::Completed),
                Some(ToolStatus::Failed),
                Some(ToolStatus::InProgress)
            ],
            "what it is doing is the spawning call's own status"
        );
    }

    /// One transcript at a time: the main agent's excludes its delegates' turns, and each
    /// delegate's includes only its own.
    #[test]
    fn the_transcript_shows_exactly_one_agent() {
        let mut c = three_greeters();

        let main: Vec<usize> = c.visible_blocks().into_iter().map(|(ix, _)| ix).collect();
        assert_eq!(
            main,
            vec![0, 1, 2, 3],
            "the prose and the three Task calls, and none of what they said"
        );

        c.viewing = Some("t754".to_string());
        assert_eq!(
            c.visible_blocks()
                .into_iter()
                .map(|(_, block)| block.clone())
                .collect::<Vec<_>>(),
            vec![ConvBlock::Agent {
                body: "Ahoy!".to_string(),
                subagent: Some(Subagent {
                    id: "t754".to_string(),
                    kind: Some("general-purpose".to_string()),
                    ..Default::default()
                }),
            }],
        );
    }

    /// The name is the spawning call's title, which the bridge writes from the task description.
    /// With no such call in the transcript there is nothing to title it with, so the kind stands
    /// in — and the raw id only where the harness named neither.
    #[test]
    fn a_subagents_name_falls_back_to_its_kind() {
        let c = three_greeters();
        assert_eq!(c.subagent_name("t751"), "Formal greeting agent");

        let mut orphan = conversation();
        orphan.apply(1, said_by("no Task call here", "t900", Some("Explore")));
        assert_eq!(orphan.subagent_name("t900"), "Explore");
        assert_eq!(
            orphan.subagents()[0].status,
            None,
            "no spawning call, no status — not a guess at one"
        );

        let mut nameless = conversation();
        nameless.apply(1, said_by("anonymous", "t901", None));
        assert_eq!(nameless.subagent_name("t901"), "t901");
    }

    /// A delegation block is the entry point to another transcript — but only once there is one.
    /// A `Task` call whose agent has not said anything yet has nothing to switch to, and the block
    /// stays inert rather than opening an empty view.
    #[test]
    fn a_delegation_is_a_way_in_only_once_its_agent_has_spoken() {
        let mut c = conversation();
        c.apply(
            1,
            task_call("t751", "Formal greeting agent", ToolStatus::InProgress),
        );
        assert!(!c.has_subagent("t751"), "spawned, and nothing said yet");

        c.apply(2, said_by("Good day.", "t751", Some("general-purpose")));
        assert!(c.has_subagent("t751"));
    }

    /// What a delegate answers with is its own, and it reaches the tab that both the reading strip
    /// and the panel's row tooltip read. A delegate the harness stamped no model on carries `None`
    /// and is drawn as nothing — never as the parent's model, which is the one case worth hovering
    /// for. `thinking` is `None` on every harness today, and the same rule applies to it.
    #[test]
    fn a_delegates_model_reaches_its_tab() {
        let mut c = conversation();
        c.apply(
            1,
            task_call("t751", "Formal greeting agent", ToolStatus::InProgress),
        );
        c.apply(
            2,
            ConvUpdate::AgentChunk {
                content: ConvContent::Text("Good day.".to_string()),
                message_id: Some("m1".to_string()),
                subagent: Some(Subagent {
                    id: "t751".to_string(),
                    kind: Some("general-purpose".to_string()),
                    model: Some("claude-haiku-4-5-20251001".to_string()),
                    thinking: None,
                }),
            },
        );
        c.apply(3, said_by("Ahoy!", "t754", Some("general-purpose")));

        let tabs = c.subagents();
        assert_eq!(tabs[0].kind.as_deref(), Some("general-purpose"));
        assert_eq!(
            tabs[0].model.as_deref(),
            Some("claude-haiku-4-5-20251001"),
            "the delegate's own model, as its lines were stamped"
        );
        assert_eq!(
            short_model_label(&c.harness, tabs[0].model.as_deref().unwrap()),
            "haiku",
            "shortened the way the composer's chip shortens the parent's"
        );
        assert_eq!(tabs[0].thinking, None, "no harness states one per delegate");
        assert_eq!(
            tabs[1].model, None,
            "a delegate the harness named no model for carries none — not the parent's"
        );
    }

    /// The panel is closed until it is asked for: a conversation's delegates are a fact worth a
    /// line, not a list that unfolds itself.
    #[test]
    fn the_subagent_panel_starts_collapsed() {
        assert!(!conversation().subagents_open);
    }

    /// A resume, or any transcript the id has gone from, must not leave the reader looking at an
    /// empty view.
    #[test]
    fn a_stale_viewing_id_falls_back_to_the_main_agent() {
        let mut c = three_greeters();
        c.viewing = Some("t999".to_string());

        assert_eq!(c.viewing_subagent(), None);
        assert_eq!(c.visible_blocks().len(), 4, "the main agent's own turns");
    }

    fn conversation() -> Conversation {
        Conversation::new(
            AgentId::generate(),
            "Claude Code".to_string(),
            "work".to_string(),
        )
    }

    fn chunk(text: &str, id: Option<&str>) -> ConvUpdate {
        ConvUpdate::AgentChunk {
            content: ConvContent::Text(text.to_string()),
            message_id: id.map(str::to_string),
            subagent: None,
        }
    }

    /// Chunks sharing a message id are one message. This is the whole reason
    /// a token stream does not produce a block per token.
    #[test]
    fn chunks_of_one_message_become_one_block() {
        let mut c = conversation();
        c.apply(1, chunk("Hel", Some("m1")));
        c.apply(2, chunk("lo", Some("m1")));

        assert_eq!(
            c.blocks,
            vec![ConvBlock::Agent {
                body: "Hello".to_string(),
                subagent: None
            }]
        );
    }

    #[test]
    fn a_new_message_id_starts_a_new_block() {
        let mut c = conversation();
        c.apply(1, chunk("first", Some("m1")));
        c.apply(2, chunk("second", Some("m2")));

        assert_eq!(
            c.blocks,
            vec![
                ConvBlock::Agent {
                    body: "first".to_string(),
                    subagent: None
                },
                ConvBlock::Agent {
                    body: "second".to_string(),
                    subagent: None
                },
            ]
        );
    }

    /// Prose and reasoning are different blocks even inside one message.
    #[test]
    fn a_thought_does_not_join_the_prose_before_it() {
        let mut c = conversation();
        c.apply(1, chunk("answering", Some("m1")));
        c.apply(
            2,
            ConvUpdate::ThoughtChunk {
                content: ConvContent::Text("pondering".to_string()),
                message_id: Some("m1".to_string()),
                subagent: None,
            },
        );

        assert_eq!(c.blocks.len(), 2);
        assert_eq!(
            c.blocks[1],
            ConvBlock::Thought {
                body: "pondering".to_string(),
                subagent: None
            }
        );
    }

    #[test]
    fn a_patch_changes_only_what_it_names() {
        let mut c = conversation();
        c.apply(
            1,
            ConvUpdate::ToolCall(ToolCallRecord {
                id: "t1".to_string(),
                title: "Bash ls".to_string(),
                kind: ToolKind::Execute,
                status: ToolStatus::InProgress,
                content: Vec::new(),
                locations: Vec::new(),
                subagent: None,
            }),
        );
        c.apply(
            2,
            ConvUpdate::ToolCallUpdate(ToolCallPatch {
                id: "t1".to_string(),
                status: Some(ToolStatus::Completed),
                ..ToolCallPatch::default()
            }),
        );

        let ConvBlock::Tool { call, .. } = &c.blocks[0] else {
            panic!("expected a tool block");
        };
        assert_eq!(call.status, ToolStatus::Completed);
        assert_eq!(call.title, "Bash ls", "a patch that named no title kept it");
        assert_eq!(call.kind, ToolKind::Execute);
    }

    /// Drawing a completed call with no beginning would invent one.
    #[test]
    fn a_patch_for_a_call_nobody_announced_is_dropped() {
        let mut c = conversation();
        c.apply(
            1,
            ConvUpdate::ToolCallUpdate(ToolCallPatch {
                id: "ghost".to_string(),
                status: Some(ToolStatus::Completed),
                ..ToolCallPatch::default()
            }),
        );
        assert!(c.blocks.is_empty());
    }

    #[test]
    fn the_ring_comes_from_the_harness_rather_than_a_constant() {
        let mut c = conversation();
        assert_eq!(c.context_pct(), None, "nothing reported yet");

        c.apply(
            1,
            ConvUpdate::Usage(UsageRecord {
                used: 100_000,
                size: 1_000_000,
                cost_usd: Some(0.25),
                model: Some("claude-opus-5".to_string()),
                spend: Some(TokenSpend {
                    input: 40_000,
                    output: 10_000,
                    thinking: 10_000,
                    cache_read: 180_000,
                    // The expected numbers changed with the spend split: `cached_tokens` is now
                    // cache *read* alone, and cache creation is counted separately.
                    cache_creation: 0,
                }),
                subagent: None,
            }),
        );

        assert_eq!(c.context_pct(), Some(10));
        assert_eq!(c.tokens(), 100_000);
        assert_eq!(c.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(c.total_tokens(), Some(240_000));
        assert_eq!(c.cached_tokens(), Some(180_000));
    }

    fn usage(used: u64, size: u64, spend: TokenSpend, subagent: Option<&str>) -> ConvUpdate {
        ConvUpdate::Usage(UsageRecord {
            used,
            size,
            cost_usd: None,
            model: None,
            spend: Some(spend),
            subagent: subagent.map(str::to_string),
        })
    }

    fn spend(input: u64, output: u64) -> TokenSpend {
        TokenSpend {
            input,
            output,
            ..TokenSpend::default()
        }
    }

    /// The two rules that decide everything the footer draws: occupancy is a level and is
    /// replaced, spend is a flow and is summed. Reading the last record's spend as the total —
    /// which is what this used to do — under-reports every report before it.
    #[test]
    fn spend_accumulates_while_occupancy_is_replaced() {
        let mut c = conversation();
        c.apply(1, usage(10_000, 200_000, spend(1_000, 100), None));
        c.apply(2, usage(30_000, 200_000, spend(2_000, 200), None));

        assert_eq!(c.tokens(), 30_000, "occupancy is the last reading");
        assert_eq!(c.context_pct(), Some(15));
        assert_eq!(c.total_tokens(), Some(3_300), "spend is every reading");
    }

    /// A subagent repeats the parent's `used`/`size` unchanged, so applying one as a fresh reading
    /// would move the ring for a turn that never touched the parent's window.
    #[test]
    fn a_subagents_report_spends_without_moving_the_ring() {
        let mut c = conversation();
        c.apply(1, usage(50_000, 200_000, spend(4_000, 400), None));
        c.apply(
            2,
            usage(50_000, 200_000, spend(9_000, 900), Some("Explore")),
        );
        c.apply(
            3,
            usage(50_000, 200_000, spend(1_000, 100), Some("Explore")),
        );
        c.apply(4, usage(50_000, 200_000, spend(2_000, 200), Some("Plan")));

        assert_eq!(
            c.tokens(),
            50_000,
            "the parent still says where the ring is"
        );
        assert_eq!(c.context_pct(), Some(25));
        assert_eq!(c.total_tokens(), Some(17_600));
        assert_eq!(
            c.subagent_spend(),
            vec![("Explore", 11_000), ("Plan", 2_200)],
            "biggest spender first, and the conversation's own entry is not among them"
        );
        assert_eq!(
            c.spend_by_subagent[""].total(),
            4_400,
            "the conversation's own spend is kept apart under its own key"
        );
    }

    /// The worst of the bugs a real capture showed: a subagent's greeting drawn inside the
    /// sentence the main agent was in the middle of. Message ids are minted per turn and collide.
    #[test]
    fn a_subagents_chunk_does_not_join_the_parents_block() {
        let mut c = conversation();
        c.apply(1, chunk("I'll delegate. ", Some("m1")));
        c.apply(
            2,
            ConvUpdate::AgentChunk {
                content: ConvContent::Text("Hello from the subagent".to_string()),
                message_id: Some("m1".to_string()),
                subagent: Some(Subagent {
                    id: "toolu_1".to_string(),
                    kind: Some("Explore".to_string()),
                    ..Default::default()
                }),
            },
        );

        assert_eq!(c.blocks.len(), 2, "two voices are two blocks");
        assert_eq!(c.blocks[0].subagent_id(), None);
        assert_eq!(c.blocks[1].subagent_id(), Some("toolu_1"));
    }

    /// The chip is short enough to read at a glance without inventing a rule for vendors whose
    /// ids do not share Claude's shape.
    #[test]
    fn only_claude_ids_are_shortened_to_their_family() {
        assert_eq!(
            short_model_label("Claude Code", "claude-haiku-4-5-20251001"),
            "haiku"
        );
        assert_eq!(
            short_model_label("Claude Code", "opus"),
            "opus",
            "nothing to cut on"
        );
        assert_eq!(
            short_model_label("Codex", "gpt-5-codex"),
            "gpt-5-codex",
            "another harness's id is its own"
        );
    }

    #[test]
    fn a_rate_limit_update_is_held_and_read_back() {
        let mut c = conversation();
        assert_eq!(c.rate_limit_five_hour_pct(), None, "nothing reported yet");

        c.apply(
            1,
            ConvUpdate::RateLimit(RateLimitRecord {
                five_hour_pct: Some(7),
                five_hour_resets_at: Some(1_788_474_600),
                seven_day_pct: Some(21),
                seven_day_resets_at: Some(1_788_796_800),
                status: "allowed".to_string(),
                overage_status: None,
                overage_reason: None,
            }),
        );

        assert_eq!(c.rate_limit_five_hour_pct(), Some(7));
    }

    #[test]
    fn a_waiting_permission_outranks_whatever_it_was_doing() {
        let mut c = conversation();
        c.apply(1, chunk("working", Some("m1")));
        assert_eq!(c.activity(), Activity::Writing);

        c.apply(
            2,
            ConvUpdate::PermissionRequest {
                request_id: "r1".to_string(),
                tool_call: ToolCallPatch::default(),
                options: Vec::new(),
            },
        );
        assert_eq!(c.activity(), Activity::NeedsYou);
    }

    #[test]
    fn a_failed_end_shows_as_failed_rather_than_ended() {
        let mut c = conversation();
        c.ended(StopReason::Failed);
        assert_eq!(c.activity(), Activity::Failed);
        assert_eq!(c.run, Run::Ended);
    }

    /// The bus promises order per agent, so a gap is a lost message.
    #[test]
    fn a_gap_in_the_sequence_is_visible() {
        let mut c = conversation();
        assert!(c.is_next(1));
        c.apply(1, chunk("a", Some("m1")));
        assert!(c.is_next(2));
        assert!(!c.is_next(4));
    }

    /// Ids are stable and increase, so an edit or a delete elsewhere in the queue never renames
    /// what a caller already holds a reference to.
    #[test]
    fn enqueue_hands_out_stable_increasing_ids() {
        let mut c = conversation();
        let first = c.enqueue("one".to_string());
        let second = c.enqueue("two".to_string());
        assert_ne!(first, second);
        assert_eq!(
            c.queued,
            vec![
                QueuedMessage {
                    id: first,
                    text: "one".to_string()
                },
                QueuedMessage {
                    id: second,
                    text: "two".to_string()
                },
            ]
        );
    }

    /// What a turn ending sends automatically: the oldest one, first in first out.
    #[test]
    fn dequeue_front_pops_the_oldest() {
        let mut c = conversation();
        c.enqueue("first".to_string());
        c.enqueue("second".to_string());

        let popped = c.dequeue_front().expect("a queued message");
        assert_eq!(popped.text, "first");
        assert_eq!(c.queued.len(), 1);
        assert_eq!(c.dequeue_front().unwrap().text, "second");
        assert_eq!(c.dequeue_front(), None, "nothing left to pop");
    }

    /// Delete, and the other half of an edit: the caller re-populates the composer with what
    /// comes back.
    #[test]
    fn remove_queued_takes_the_named_entry_back_out() {
        let mut c = conversation();
        let keep = c.enqueue("keep".to_string());
        let drop = c.enqueue("drop this".to_string());

        let text = c.remove_queued(drop).expect("the entry existed");
        assert_eq!(text, "drop this");
        assert_eq!(
            c.queued,
            vec![QueuedMessage {
                id: keep,
                text: "keep".to_string()
            }]
        );
        assert_eq!(c.remove_queued(drop), None, "already removed");
    }

    /// The mechanics `app.rs`'s auto-send glue relies on: a turn ending flips `run` to `Idle`,
    /// and the front of the queue is then a plain pop away — the state layer's half of "send the
    /// next queued prompt when a turn ends", which is as far as this layer's own test can reach.
    #[test]
    fn a_turn_ending_leaves_the_queue_ready_to_drain() {
        let mut c = conversation();
        c.enqueue("next up".to_string());
        c.apply(
            1,
            ConvUpdate::TurnEnded {
                stop_reason: StopReason::EndTurn,
                error: None,
            },
        );

        assert_eq!(c.run, Run::Idle);
        assert_eq!(c.dequeue_front().unwrap().text, "next up");
    }
}
