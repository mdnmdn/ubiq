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

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};

use gpui::{Pixels, ScrollHandle, px};
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
    /// How many permission requests this delegate is blocked on. Non-zero is what puts `need you`
    /// on its row, in place of what it would otherwise say it was doing: a delegate waiting on a
    /// human is not doing anything, and the question is the more useful of the two readings.
    pub waiting: usize,
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
///
/// `tool_call` is a *patch*, and upstream guarantees only its id: everything else may be absent.
/// So what a prompt says about the operation is read off the call already in the transcript —
/// [`Conversation::tool_block_index`] does that join — and this carries only what the request itself
/// added.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub request_id: String,
    pub tool_call: ToolCallPatch,
    pub options: Vec<PermissionOption>,
}

impl Pending {
    /// The first option that reads as going ahead, or as refusing — what the keyboard answers
    /// with. `kind` is a hint, so this picks by hint and echoes the `option_id` it found: nothing
    /// here interprets an id.
    pub fn option_for(&self, allow: bool) -> Option<&PermissionOption> {
        self.options
            .iter()
            .find(|option| option.kind.allows() == allow)
    }

    /// The option that goes ahead *and* is remembered — the "all" of yes / no / all.
    ///
    /// `None` where the harness offered no such reading, and then nothing is drawn: a third
    /// button that answered with the plain allow would be a control that lies about lasting.
    /// **The harness remembers it, not Ubiq** — this is one more opaque `option_id` echoed back.
    pub fn always_option(&self) -> Option<&PermissionOption> {
        self.options
            .iter()
            .find(|option| option.kind.allows() && option.kind.remembers())
    }
}

/// A prompt typed while a turn was already running, held until it ends.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedMessage {
    pub id: u64,
    pub text: String,
}

/// One file the composer will hand this conversation with the next prompt.
///
/// **Held here rather than in the composer's own field**, beside [`Conversation::draft`] and for
/// its reason: unsent composer content belongs to the conversation, so it follows it from one
/// surface to another rather than being lost when a tab is switched or a slot is handed on.
///
/// Nothing is read from a disk to build one: the size is whatever the picker already reported for
/// that node, which is what the host said when it listed the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Stable per conversation, on the same reasoning as [`QueuedMessage::id`]: an element id and
    /// a removal name the same entry even as others are added or removed around it.
    pub id: u64,
    /// Project-relative — the one path shape the interface holds.
    pub path: String,
    /// How big it is, where anything said so. `None` is not guessed at: a tag with no size reads
    /// as an ordinary file rather than as a small one.
    pub size: Option<u64>,
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
    /// The five-word reading of what this conversation is about, drawn as the title's tooltip
    /// wherever the name is printed. `None` until something names the conversation: a title says
    /// which one this is, and the summary is what it took a whole exchange to learn.
    pub summary: Option<String>,
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
    /// Every permission the agent is waiting on, in the order the requests arrived.
    ///
    /// **A list, not a slot.** Upstream may have several requests outstanding at once and expects
    /// every one of them answered; there are no timeouts, so a request this dropped on the floor
    /// would deadlock the turn with nothing on screen to say so. Keyed by `request_id`: a second
    /// request under an id already here replaces it rather than queueing a duplicate.
    pub pending: Vec<Pending>,
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
    /// Which collapsed runs of same-kind tool calls the reader has opened, keyed by the id of the
    /// run's first call. A run of `READ`s is drawn as its last card plus one row standing for the
    /// rest, and this says which of those rows have been asked to show what they stand for. Keyed
    /// by call id rather than by block index because a run's position is a property of the
    /// transcript's current shape and its first call's id is not.
    pub open_groups: HashSet<String>,
    /// Whether the subagent panel is open. Collapsed by default and per conversation, beside
    /// [`Self::viewing`] and for its reason: several conversations are on screen at once, and each
    /// reader opens the ones they are following.
    pub subagents_open: bool,
    /// Prompts typed while a turn was already running, held until it ends. A stable
    /// per-conversation id per entry, so an edit or a delete names the right one even if others
    /// are added or removed around it.
    pub queued: Vec<QueuedMessage>,
    next_queued_id: u64,
    /// Files picked for the next prompt, in the order they were picked. Unsent composer content,
    /// so it sits beside [`Self::draft`] and [`Self::queued`] rather than being indexed by the
    /// composer slot that happens to be drawing it.
    pub attached: Vec<Attachment>,
    next_attached_id: u64,

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
            summary: None,
            usage: None,
            spend: None,
            spend_by_subagent: BTreeMap::new(),
            rate_limit: None,
            run: Run::Idle,
            stop_reason: None,
            config: Vec::new(),
            plan: Vec::new(),
            pending: Vec::new(),
            error: None,
            accepts_input: true,
            draft: String::new(),
            launched: false,
            chosen: BTreeMap::new(),
            open_config: None,
            viewing: None,
            open_groups: HashSet::new(),
            subagents_open: false,
            queued: Vec::new(),
            next_queued_id: 0,
            attached: Vec::new(),
            next_attached_id: 0,
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
                waiting: self.pending_count(Some(id)),
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
        if !self.pending.is_empty() {
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
                let request = Pending {
                    request_id,
                    tool_call,
                    options,
                };
                // Keyed by id, appended otherwise: arrival order is what the transcript and the
                // keyboard both read as "the oldest one still waiting".
                match self
                    .pending
                    .iter_mut()
                    .find(|held| held.request_id == request.request_id)
                {
                    Some(held) => *held = request,
                    None => self.pending.push(request),
                }
            }

            ConvUpdate::TurnEnded { stop_reason, error } => {
                self.open = None;
                self.run = Run::Idle;
                self.stop_reason = Some(stop_reason);
                self.error = error;
            }
        }
    }

    /// Ubiq has read the opening exchange and named this conversation.
    ///
    /// The same field `ConvUpdate::Title` writes, because it is the same fact from the other
    /// source — whichever spoke last is the name.
    pub fn name(&mut self, title: String, summary: Option<String>) {
        self.title = Some(title);
        self.summary = summary;
    }

    /// The harness has gone.
    pub fn ended(&mut self, stop_reason: StopReason) {
        self.open = None;
        self.pending.clear();
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
        self.pending.clear();
    }

    /// Forget one request, because it has been answered. Idempotent: an answer that raced the
    /// harness's own withdrawal of the request finds nothing and does nothing.
    pub fn answered(&mut self, request_id: &str) {
        self.pending.retain(|held| held.request_id != request_id);
    }

    /// The oldest request still waiting — what the keyboard answers, and what the strip above the
    /// footer names when several are up.
    pub fn oldest_pending(&self) -> Option<&Pending> {
        self.pending.first()
    }

    /// Which block a tool call is drawn as, if the transcript holds it at all. The join a
    /// permission prompt needs: a request carries a call id and nothing else it can rely on.
    pub fn tool_block_index(&self, id: &str) -> Option<usize> {
        self.tools.get(id).copied()
    }

    /// Whose transcript a request is asking about — the subagent that raised it, or `None` for
    /// the conversation's own turn.
    ///
    /// Read off the block the call is drawn as, which is the same join
    /// [`Self::tool_block_index`] answers: a request names a call id, and the call's block is the
    /// only thing that knows who was speaking. A request naming a call this transcript has never
    /// seen belongs to nobody, and reads as the main agent's rather than being filed under a
    /// guess.
    pub fn pending_subagent(&self, pending: &Pending) -> Option<&str> {
        let ix = self.tool_block_index(&pending.tool_call.id)?;
        self.blocks.get(ix)?.subagent_id()
    }

    /// Where a request is drawn: whose transcript, and which block of it. `None` for the block
    /// where the transcript does not hold the call — the prompt degrades to the self-contained
    /// one at the end, which is where a reader is sent instead.
    ///
    /// The one place the routing behind the "needs you" strip is resolved. Clicking the strip
    /// switches to this subagent and scrolls to this block, and both answers have to be the same
    /// reading or the strip sends the reader somewhere the prompt is not.
    pub fn pending_route(&self, pending: &Pending) -> (Option<String>, Option<usize>) {
        let ix = self.tool_block_index(&pending.tool_call.id);
        let who = ix
            .and_then(|ix| self.blocks.get(ix))
            .and_then(ConvBlock::subagent_id)
            .map(str::to_string);
        (who, ix)
    }

    /// How many requests whoever is being read is waiting on — `None` being the main agent's own
    /// turn. What a delegate's row in the switcher marks itself with.
    pub fn pending_count(&self, subagent: Option<&str>) -> usize {
        self.pending
            .iter()
            .filter(|held| self.pending_subagent(held) == subagent)
            .count()
    }

    /// What one subagent **type** has spent — its total and the cached part of it — for the
    /// footer of a delegate's transcript.
    ///
    /// Keyed by type and not by instance because that is the only grain the wire carries:
    /// `UsageRecord::subagent` is deliberately a type, so two `general-purpose` delegates share
    /// one bucket. The footer says so on hover rather than drawing a type's total as one
    /// instance's.
    pub fn subagent_tokens(&self, kind: &str) -> Option<(u64, u64)> {
        let spend = self.spend_by_subagent.get(kind)?;
        (spend.total() > 0).then(|| (spend.total(), spend.cached()))
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

    /// Attach a file to the next prompt, and say which entry it became.
    ///
    /// **A path already attached is not attached twice**, and answers `None`: two tags for one
    /// file would be two mentions of it in the prompt, and a remove that only half worked. The
    /// size is refreshed on the entry that is already there, since the picker that just reported
    /// it has read the folder more recently than whatever attached it first.
    pub fn attach(&mut self, path: String, size: Option<u64>) -> Option<u64> {
        if let Some(existing) = self.attached.iter_mut().find(|file| file.path == path) {
            existing.size = size;
            return None;
        }
        let id = self.next_attached_id;
        self.next_attached_id += 1;
        self.attached.push(Attachment { id, path, size });
        Some(id)
    }

    /// Take one attachment back off, by id — a tag's own dismiss control.
    pub fn detach(&mut self, id: u64) -> Option<Attachment> {
        let ix = self.attached.iter().position(|file| file.id == id)?;
        Some(self.attached.remove(ix))
    }

    /// Drop every attachment — what a prompt leaving consumes, alongside the draft.
    pub fn clear_attached(&mut self) {
        self.attached.clear();
    }

    /// What actually goes on the wire: what was typed, with every attachment named after it as
    /// `@path`.
    ///
    /// The mentions come *after* the prose because the sentence around the paths is usually
    /// written first, which is also the order the picker used to append them in. `@path`,
    /// project-relative, is the one path shape the interface holds and the shape every harness
    /// reads a file reference in — nothing new crosses the bus for this, so a prompt with
    /// attachments is a `PromptAgent` like any other.
    pub fn compose_prompt(&self, typed: &str) -> String {
        let mut out = typed.trim().to_string();
        for file in &self.attached {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push('@');
            out.push_str(&file.path);
        }
        out
    }

    /// Toggle a tool block's detail.
    pub fn toggle_tool(&mut self, id: &str) {
        if let Some(ConvBlock::Tool { open, .. }) =
            self.tools.get(id).map(|ix| &mut self.blocks[*ix])
        {
            *open = !*open;
        }
    }

    /// Open or shut one collapsed run of tool calls, named by its first call's id.
    pub fn toggle_group(&mut self, id: &str) {
        if !self.open_groups.remove(id) {
            self.open_groups.insert(id.to_string());
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

// ── where a reader was left, per transcript ────────────────────────────

/// Which transcript a scroll position belongs to: the conversation, and which of its delegates.
///
/// The delegate is part of the key because switching to a subagent is arriving at a *different*
/// transcript, not moving within one — a reader sent to a delegate's tail and back must find the
/// main agent where they left it.
pub type TranscriptKey = (AgentId, Option<String>);

/// How close to the tail still counts as reading the tail. A few pixels of slack, because a
/// wheel notch that lands one pixel short is not a reader who has scrolled away.
const TAIL_SLACK: Pixels = px(24.);

/// Above this many blocks the transcript stops building what is off screen. Below it every block
/// is built every frame, which is both cheaper than the bookkeeping and exact on the first frame.
const WINDOW_MIN: usize = 40;

/// How far beyond the viewport a block is still built, so a wheel notch lands on drawn content
/// rather than on a placeholder waiting for the next frame.
const WINDOW_MARGIN: Pixels = px(2_000.);

/// What one composer slot's transcript remembers between frames.
///
/// A slot, not a conversation: the handle belongs to the element the pool built, and a slot shows
/// one transcript at a time. What is per *transcript* is the position, and that is what
/// [`Self::saved`] holds — so a slot moved from an agent to its delegate and back restores both.
///
/// Every field is interior-mutable because `render` holds `&AppState`: there is no mutable path
/// to this from inside an element, and these are readings of the last frame rather than state the
/// application owns.
#[derive(Default)]
pub struct TranscriptScroll {
    pub handle: ScrollHandle,
    /// The tail signature the handle was last followed to the bottom for. What keeps the follow
    /// from fighting the reader: the transcript follows only when the tail actually moved.
    followed: Cell<u64>,
    /// Which transcript the handle currently holds a position for.
    showing: RefCell<Option<TranscriptKey>>,
    /// Where the reader was in each transcript this slot has shown.
    saved: RefCell<HashMap<TranscriptKey, Pixels>>,
    /// A block this slot has been asked to bring into view. Taken once and cleared: a request to
    /// go somewhere is answered, not re-answered every frame afterwards.
    target: Cell<Option<usize>>,
    /// Whether the last frame was painted away from the tail. Read from the handle before
    /// anything moves it, and what both the follow and the jump button ask.
    away: Cell<bool>,
    /// Whether this frame is drawing the same transcript the last one did.
    ///
    /// A frame that has just switched transcripts is measuring the *previous* one: the scroll
    /// handle's record of where each child was painted still describes the transcript that has
    /// gone. So a request to be taken to a block waits for the frame after the switch, which is
    /// the first frame whose measurements are of the transcript the block is in.
    settled: Cell<bool>,
}

impl TranscriptScroll {
    /// Read the last frame's position, then put the handle where this frame's transcript wants
    /// it. Called once per frame, before the children are built.
    ///
    /// Three cases, in this order. A **different transcript** than the last frame's saves where
    /// the outgoing one was and restores where this one was, and follows the tail only for one
    /// never seen before. The **same transcript with a moved tail** follows it, but only for a
    /// reader already at the tail — which is the whole of "preserve the scroll": a delegate three
    /// screens up stays three screens up while the agent below it keeps writing. Anything else
    /// leaves the handle alone.
    pub fn sync(&self, key: TranscriptKey, signature: u64) {
        self.away.set(self.away_from_tail());
        let mut showing = self.showing.borrow_mut();
        if showing.as_ref() != Some(&key) {
            if let Some(previous) = showing.take() {
                self.saved
                    .borrow_mut()
                    .insert(previous, self.handle.offset().y);
            }
            *showing = Some(key.clone());
            self.settled.set(false);
            // The incoming transcript's tail is not news, whoever put it there.
            self.followed.set(signature);
            let saved = self.saved.borrow().get(&key).copied();
            match saved {
                Some(y) => {
                    self.handle
                        .set_offset(gpui::point(self.handle.offset().x, y));
                    self.away.set(true);
                }
                // Somewhere to be taken to outranks the tail: gpui applies `scroll_to_bottom`
                // *after* a scroll-to-item, so asking for both in one frame is asking for the
                // bottom.
                None if self.target.get().is_none() => {
                    self.handle.scroll_to_bottom();
                    self.away.set(false);
                }
                None => {}
            }
            return;
        }
        self.settled.set(true);
        if self.followed.get() != signature {
            self.followed.set(signature);
            if !self.away.get() && self.target.get().is_none() {
                self.handle.scroll_to_bottom();
            }
        }
    }

    /// Whether the reader is reading something other than the tail — what draws the jump button,
    /// and what stops an arriving chunk from dragging them back down.
    ///
    /// Measured from the last frame the handle painted: the offset is zero or negative and sits
    /// at `-max_offset` at the bottom, so the two summed are the distance still to go. A slot
    /// that has never painted has no maximum and is not away from anything.
    pub fn away_from_tail(&self) -> bool {
        let max = self.handle.max_offset().y;
        max > px(0.) && self.handle.offset().y + max > TAIL_SLACK
    }

    /// What the last frame decided, for whoever is drawing this frame's overlay.
    pub fn away(&self) -> bool {
        self.away.get()
    }

    /// Ask for a block to be brought into view on the next frame — how the "needs you" strip
    /// arrives at the prompt it named.
    pub fn request(&self, block: usize) {
        self.target.set(Some(block));
    }

    /// Take the block asked for, if one was and this frame can answer it. Answered once.
    ///
    /// Held back on the frame that switched transcripts — see [`Self::settled`] — so a jump to a
    /// delegate's prompt lands on the delegate's block rather than wherever that child index
    /// happened to be in the transcript the reader just left.
    pub fn take_request(&self) -> Option<usize> {
        self.settled.get().then(|| self.target.take()).flatten()
    }

    /// Whether a block is still waiting to be scrolled to. The frame that switched transcripts
    /// cannot answer one, so it has to ask for the frame that can.
    pub fn request_held(&self) -> bool {
        self.target.get().is_some()
    }

    /// Put the reader back on the tail, and follow it again from here.
    pub fn to_tail(&self) {
        self.handle.scroll_to_bottom();
        self.away.set(false);
    }

    /// Whether a transcript of this many children is long enough to be worth windowing.
    pub fn windows(&self, children: usize) -> bool {
        children > WINDOW_MIN && self.handle.bounds().size.height > px(0.)
    }

    /// Where a child of the last frame was painted, for deciding whether to build it again.
    /// `None` for a child that frame did not have.
    pub fn child_bounds(&self, ix: usize) -> Option<gpui::Bounds<Pixels>> {
        self.handle.bounds_for_item(ix)
    }

    /// Whether a child painted at these bounds is close enough to the viewport to build.
    ///
    /// **The two are in different spaces, and that is the whole of this function.** A scroll
    /// handle records each child where it was *laid out*, with the container's own scroll not yet
    /// applied, while the viewport is where the container sits on screen. So the visible window in
    /// the children's space is the viewport shifted by the offset — the same arithmetic
    /// `ScrollHandle::top_item` does, deliberately, because a second reading of it that drifted
    /// would window the wrong blocks and blank the ones being read.
    pub fn near_viewport(&self, bounds: gpui::Bounds<Pixels>) -> bool {
        let viewport = self.handle.bounds();
        let offset = self.handle.offset().y;
        let top = viewport.top() - offset - WINDOW_MARGIN;
        let bottom = viewport.bottom() - offset + WINDOW_MARGIN;
        bounds.bottom() >= top && bounds.top() <= bottom
    }
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

    /// A request carrying nothing but its own id and an empty patch — the shape upstream
    /// guarantees, and the one a prompt has to survive.
    fn permission(request_id: &str) -> ConvUpdate {
        ConvUpdate::PermissionRequest {
            request_id: request_id.to_string(),
            tool_call: ToolCallPatch::default(),
            options: Vec::new(),
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

    /// The footer of a delegate's transcript, and the one thing about it worth stating twice: the
    /// wire keys spend by subagent **type**, so two `general-purpose` instances share one bucket.
    /// Drawing a type's total as one instance's would over-report it by however many siblings it
    /// had, which is why the footer says so on hover rather than pretending otherwise.
    #[test]
    fn subagent_tokens_are_a_types_bucket_rather_than_an_instances() {
        let mut c = conversation();
        c.apply(1, usage(50_000, 200_000, spend(4_000, 400), None));
        // Two instances of one type, reporting separately.
        c.apply(
            2,
            usage(50_000, 200_000, spend(9_000, 900), Some("general-purpose")),
        );
        c.apply(
            3,
            usage(50_000, 200_000, spend(1_000, 100), Some("general-purpose")),
        );

        assert_eq!(
            c.subagent_tokens("general-purpose"),
            Some((11_000, 0)),
            "both instances summed into the one bucket their type has"
        );
        assert_eq!(
            c.subagent_tokens("Explore"),
            None,
            "a type that never reported has nothing to draw — not a zero it made up"
        );
    }

    /// The cached half of the reading is cache *read* alone: context re-used is saved, context
    /// newly written is paid for once. And a bucket that exists but counted nothing reads `None`,
    /// so the footer draws nothing rather than a row of zeroes.
    #[test]
    fn subagent_tokens_count_cache_read_as_the_cached_part() {
        let mut c = conversation();
        c.apply(
            1,
            usage(
                50_000,
                200_000,
                TokenSpend {
                    input: 1_000,
                    output: 100,
                    cache_read: 9_000,
                    cache_creation: 500,
                    ..TokenSpend::default()
                },
                Some("Explore"),
            ),
        );
        c.apply(
            2,
            usage(50_000, 200_000, TokenSpend::default(), Some("Plan")),
        );

        assert_eq!(
            c.subagent_tokens("Explore"),
            Some((10_600, 9_000)),
            "every token billed, and the cache-read part of it"
        );
        assert_eq!(
            c.subagent_tokens("Plan"),
            None,
            "a bucket that was opened and counted nothing draws nothing"
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

        c.apply(2, permission("r1"));
        assert_eq!(c.activity(), Activity::NeedsYou);
    }

    /// Upstream may have several requests open at once and expects every one answered, so a
    /// second must not push the first off the screen — and answering one must not clear the other.
    #[test]
    fn two_permissions_are_both_held_and_answered_one_at_a_time() {
        let mut c = conversation();
        c.apply(1, permission("r1"));
        c.apply(2, permission("r2"));

        let ids: Vec<&str> = c.pending.iter().map(|p| p.request_id.as_str()).collect();
        assert_eq!(ids, vec!["r1", "r2"]);
        assert_eq!(
            c.oldest_pending().map(|p| p.request_id.as_str()),
            Some("r1")
        );

        c.answered("r1");
        let ids: Vec<&str> = c.pending.iter().map(|p| p.request_id.as_str()).collect();
        assert_eq!(ids, vec!["r2"]);
        assert_eq!(c.activity(), Activity::NeedsYou);

        c.answered("r2");
        assert!(c.pending.is_empty());
        assert_ne!(c.activity(), Activity::NeedsYou);
    }

    /// The same id twice is the harness restating one request, not a second one.
    #[test]
    fn a_repeated_request_id_replaces_rather_than_queues() {
        let mut c = conversation();
        c.apply(1, permission("r1"));
        c.apply(2, permission("r1"));
        assert_eq!(c.pending.len(), 1);
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

    /// One tag per file, however many times it is picked — and the second pick is what refreshes
    /// the size, since it read the folder more recently.
    #[test]
    fn attaching_the_same_path_twice_keeps_one_tag() {
        let mut c = conversation();
        let first = c.attach("src/main.rs".to_string(), Some(10));
        let again = c.attach("src/main.rs".to_string(), Some(20));

        assert_eq!(first, Some(0));
        assert_eq!(again, None, "already attached");
        assert_eq!(c.attached.len(), 1);
        assert_eq!(c.attached[0].size, Some(20));
    }

    /// Ids are stable, so a remove names the entry the tag was drawn for and nothing else moves
    /// under it.
    #[test]
    fn detach_takes_the_named_attachment_back_out() {
        let mut c = conversation();
        let keep = c.attach("a.rs".to_string(), None).expect("attached");
        let drop = c.attach("b.rs".to_string(), Some(1)).expect("attached");

        let gone = c.detach(drop).expect("the entry existed");
        assert_eq!(gone.path, "b.rs");
        assert_eq!(c.attached.len(), 1);
        assert_eq!(c.attached[0].id, keep);
        assert_eq!(c.detach(drop), None, "already removed");
    }

    /// What sending does: the mentions are composed into the one `PromptAgent` text, and the
    /// attachments are consumed with the draft rather than carried into the next turn.
    #[test]
    fn attachments_compose_into_the_prompt_and_are_cleared_on_send() {
        let mut c = conversation();
        c.attach("src/lib.rs".to_string(), Some(400 * 1024));
        c.attach("README.md".to_string(), None);

        assert_eq!(
            c.compose_prompt("  review these  "),
            "review these @src/lib.rs @README.md"
        );
        assert_eq!(
            c.compose_prompt(""),
            "@src/lib.rs @README.md",
            "attachments alone are still something to send"
        );

        c.clear_attached();
        assert!(c.attached.is_empty());
        assert_eq!(c.compose_prompt("plain"), "plain");
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
