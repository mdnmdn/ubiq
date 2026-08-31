//! The fixtures the scaffold is populated with.
//!
//! Everything the workbench shows today comes from here. When the coordinator arrives, these
//! constructors are what its messages replace — nothing else in `ui/` knows the data is invented.

use crate::theme::ThemeId;

use super::chat::{Block, Chat, ChatMessage, ChatState, DiffLine, ToolCall, ToolKind};
use super::editor::{EditorPaneState, FileLanguage, OpenFile};
use super::explorer::{ExplorerState, FileNode, GitStatus};
use super::workbench::{Project, RailMode, WorkbenchState};

pub fn workbench(project: usize) -> WorkbenchState {
    let projects = projects();
    let project = project.min(projects.len().saturating_sub(1));

    WorkbenchState {
        rail_mode: RailMode::Ide,
        show_left: true,
        show_bottom: true,
        show_right: true,
        theme_id: ThemeId::Dark,
        open_menu: None,
        projects,
        project,
        project_filter: String::new(),
        pending_close: None,
        branches: vec![
            "main".to_string(),
            "feat/gpui-shell".to_string(),
            "fix/pane-resize".to_string(),
        ],
        branch: 0,
        ahead: 2,
        behind: 0,
        modified: 1,
        untracked: 1,
        conflicts: 1,
    }
}

/// The projects the picker offers: the open ones first, then the ones only remembered. Each keeps
/// its own swatch, which is what identifies it everywhere in the window.
pub fn projects() -> Vec<Project> {
    vec![
        Project {
            name: "agent-manager".to_string(),
            path: "~/dev/agent-manager".to_string(),
            colour: 0,
            open: true,
            terminals: 3,
            when: "now".to_string(),
        },
        Project {
            name: "ubiq".to_string(),
            path: "~/dev/ubiq".to_string(),
            colour: 1,
            open: true,
            terminals: 0,
            when: "12m".to_string(),
        },
        Project {
            name: "hire-mate".to_string(),
            path: "~/dev/hire-mate".to_string(),
            colour: 2,
            open: false,
            terminals: 0,
            when: "yst".to_string(),
        },
        Project {
            name: "multica".to_string(),
            path: "~/dev/multica".to_string(),
            colour: 3,
            open: false,
            terminals: 0,
            when: "3d".to_string(),
        },
        Project {
            name: "gpui-playground".to_string(),
            path: "~/dev/gpui-playground".to_string(),
            colour: 4,
            open: false,
            terminals: 0,
            when: "2w".to_string(),
        },
    ]
}

pub fn explorer() -> ExplorerState {
    ExplorerState::new(vec![
        FileNode::dir(
            "src",
            GitStatus::Clean,
            true,
            vec![
                FileNode::dir(
                    "src/panels",
                    GitStatus::Clean,
                    true,
                    vec![
                        FileNode::file("src/panels/AgentTerminal.tsx", GitStatus::Modified),
                        FileNode::file("src/panels/AgentTerminal.test.tsx", GitStatus::Untracked),
                        FileNode::file("src/panels/SidebarHost.tsx", GitStatus::Clean),
                    ],
                ),
                FileNode::dir(
                    "src/state",
                    GitStatus::Clean,
                    true,
                    vec![FileNode::file("src/state/sessions.ts", GitStatus::Conflict)],
                ),
                FileNode::file("src/main.tsx", GitStatus::Clean),
            ],
        ),
        FileNode::dir(
            "src-tauri",
            GitStatus::Clean,
            false,
            vec![
                FileNode::file("src-tauri/src/lib.rs", GitStatus::Clean),
                FileNode::file("src-tauri/tauri.conf.json", GitStatus::Clean),
            ],
        ),
        FileNode::dir("node_modules", GitStatus::Ignored, false, vec![]),
        FileNode::file("dist", GitStatus::Ignored),
        FileNode::file("package.json", GitStatus::Staged),
        FileNode::file("README.md", GitStatus::Clean),
    ])
}

pub fn editor() -> EditorPaneState {
    EditorPaneState::new(vec![
        OpenFile {
            name: "AgentTerminal.tsx".to_string(),
            path: "src/panels/AgentTerminal.tsx".to_string(),
            language: FileLanguage::Tsx,
            git: GitStatus::Modified,
            dirty: true,
            source: AGENT_TERMINAL_TSX.to_string(),
        },
        OpenFile {
            name: "sessions.ts".to_string(),
            path: "src/state/sessions.ts".to_string(),
            language: FileLanguage::TypeScript,
            git: GitStatus::Conflict,
            dirty: false,
            source: SESSIONS_TS.to_string(),
        },
        OpenFile {
            name: "tauri.conf.json".to_string(),
            path: "src-tauri/tauri.conf.json".to_string(),
            language: FileLanguage::Json,
            git: GitStatus::Clean,
            dirty: false,
            source: TAURI_CONF_JSON.to_string(),
        },
    ])
}

/// The terminal dock's tabs. Each becomes a pane, which is what the dock actually shows.
pub fn pane_titles() -> &'static [&'static str] {
    &["pnpm tauri dev", "claude \u{2014} agent", "git"]
}

pub fn chat() -> ChatState {
    ChatState::new(
        vec![
            Chat {
                id: 1,
                title: "Fix terminal resize after panel collapse".to_string(),
                when: "2m".to_string(),
                messages: opening_thread(),
            },
            Chat {
                id: 2,
                title: "Migrate session store to persist v2".to_string(),
                when: "1h".to_string(),
                messages: Vec::new(),
            },
            Chat {
                id: 3,
                title: "Draft 0.3.1 release notes".to_string(),
                when: "yst".to_string(),
                messages: Vec::new(),
            },
        ],
        41_200.0,
    )
}

fn opening_thread() -> Vec<ChatMessage> {
    vec![
        ChatMessage::User(
            "The xterm panel stops resizing after I collapse and reopen the sidebar. Find where \
             the fit call is getting dropped."
                .to_string(),
        ),
        ChatMessage::Assistant(vec![
            Block::Tool(ToolCall {
                kind: ToolKind::Read,
                target: "panels/AgentTerminal.tsx".to_string(),
                meta: "142 ln".to_string(),
                expanded: false,
                body: vec![
                    "40  useEffect(() => {".to_string(),
                    "41    if (!visible) return;".to_string(),
                    "42    const ro = new ResizeObserver(() => fitAddon.fit());".to_string(),
                    "43    ro.observe(hostRef.current!);".to_string(),
                    "44    return () => ro.disconnect();".to_string(),
                    "45  }, [visible]);".to_string(),
                ],
                diff: Vec::new(),
            }),
            Block::Markdown(
                "**Found it.** The host stays mounted with `hidden`, so the observer re-attaches \
                 while the box still measures 0\u{d7}0 \u{2014} `fit()` runs once against nothing \
                 and never fires again."
                    .to_string(),
            ),
            Block::Tool(ToolCall {
                kind: ToolKind::Edit,
                target: "panels/AgentTerminal.tsx".to_string(),
                meta: "+4 \u{2212}1".to_string(),
                expanded: true,
                body: Vec::new(),
                diff: vec![
                    DiffLine::remove("const ro = new ResizeObserver(() => fitAddon.fit());"),
                    DiffLine::add("const ro = new ResizeObserver(([entry]) => {"),
                    DiffLine::add("  if (entry.contentRect.width === 0) return;"),
                    DiffLine::add("  fitAddon.fit();"),
                    DiffLine::add("});"),
                ],
            }),
            Block::Tool(ToolCall {
                kind: ToolKind::Bash,
                target: "pnpm test panels".to_string(),
                meta: "exit 0".to_string(),
                expanded: false,
                body: vec![
                    "PASS  src/panels/AgentTerminal.test.tsx".to_string(),
                    "  \u{2713} refits once the host has width (18 ms)".to_string(),
                    "Tests: 1 passed, 1 total".to_string(),
                ],
                diff: Vec::new(),
            }),
            Block::Markdown(
                "The observer now ignores the zero-width measurement and fits on the first real \
                 one. Want me to add the same guard to `SidebarHost`?"
                    .to_string(),
            ),
        ]),
    ]
}

// ── File fixtures ───────────────────────────────────────────────────

const AGENT_TERMINAL_TSX: &str = r##"import { useEffect, useMemo, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

import "@xterm/xterm/css/xterm.css";

type Props = {
  paneId: string;
  visible: boolean;
};

const THEME = {
  background: "#121216",
  foreground: "#e8e8ed",
  cursor: "#5b8def",
};

export function AgentTerminal({ paneId, visible }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);

  const term = useMemo(
    () =>
      new Terminal({
        fontFamily: "Menlo, monospace",
        fontSize: 12,
        theme: THEME,
        allowProposedApi: true,
      }),
    [],
  );

  // One fit addon per terminal - it caches the last measured cell size.
  const fitAddon = useMemo(() => new FitAddon(), []);

  // refit whenever the host box actually has width - a 0x0
  // measurement lands while the sidebar is still collapsing.
  useEffect(() => {
    if (!visible) return;
    const ro = new ResizeObserver(([entry]) => {
      if (entry.contentRect.width === 0) return;
      fitAddon.fit();
    });
    ro.observe(hostRef.current!);
    return () => ro.disconnect();
  }, [visible]);

  return (
    <div className="term-host" hidden={!visible} ref={hostRef} />
  );
}
"##;

const SESSIONS_TS: &str = r##"import { create } from "zustand";
import { persist } from "zustand/middleware";

export type Session = {
  id: string;
  name: string;
  homeFolder: string;
  createdAt: string;
};

type SessionStore = {
  sessions: Session[];
  attached: string | null;
  attach: (id: string) => void;
  detach: () => void;
};

export const useSessions = create<SessionStore>()(
  persist(
    (set) => ({
      sessions: [],
      attached: null,
      attach: (id) => set({ attached: id }),
      detach: () => set({ attached: null }),
    }),
    { name: "ubiq.sessions" },
  ),
);
"##;

const TAURI_CONF_JSON: &str = r##"{
  "productName": "ubiq",
  "version": "0.3.1",
  "identifier": "dev.ubiq.app",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "title": "Ubiq",
        "width": 1440,
        "height": 900
      }
    ]
  }
}
"##;
