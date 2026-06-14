import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import { bus } from './bus.js';

const AGENT_COLORS = {
  claude: 'var(--c-claude)',
  gemini: 'var(--c-gemini)',
  codex: 'var(--c-codex)',
  opencode: 'var(--c-opencode)',
  copilot: 'var(--c-copilot)',
};

function getAgentColor(agentType) {
  return AGENT_COLORS[agentType] || 'var(--accent)';
}

class WorkspacePane {
  constructor(workspaceInfo, container) {
    this.id = workspaceInfo.id;
    this.sessionId = workspaceInfo.session_id;
    this.agentType = workspaceInfo.agent_type;
    this.running = true;
    this.terminal = null;
    this.fitAddon = null;

    this.createElement(container);
    this.initTerminal();
    this.setupBusListeners();
  }

  createElement(container) {
    this.element = document.createElement('div');
    this.element.className = 'term-card';
    this.element.dataset.workspaceId = this.id;

    const head = document.createElement('div');
    head.className = 'term-head';

    const chip = document.createElement('span');
    chip.className = 'agent-chip';
    chip.innerHTML = `<span class="sw" style="background:${getAgentColor(this.agentType)}"></span>${this.agentType}`;

    const dot = document.createElement('span');
    dot.style.color = 'var(--faint)';
    dot.textContent = '·';

    this.pidSpan = document.createElement('span');
    this.pidSpan.textContent = 'starting…';

    const spacer = document.createElement('div');
    spacer.className = 'spacer';

    this.statusSpan = document.createElement('span');
    this.statusSpan.style.color = 'var(--ok)';
    this.statusSpan.innerHTML = '● live';

    head.appendChild(chip);
    head.appendChild(dot);
    head.appendChild(this.pidSpan);
    head.appendChild(spacer);
    head.appendChild(this.statusSpan);

    this.mountElement = document.createElement('div');
    this.mountElement.className = 'term-mount';

    this.element.appendChild(head);
    this.element.appendChild(this.mountElement);
    container.appendChild(this.element);
  }

  initTerminal() {
    this.terminal = new Terminal({
      cursorBlink: true,
      fontSize: 13,
      fontFamily: '"SF Mono","JetBrains Mono",ui-monospace,Menlo,monospace',
      lineHeight: 1.35,
      theme: window.Ubiq ? window.Ubiq.termTheme() : {
        background: '#14161b',
        foreground: '#cdd2da',
        cursor: '#7aa2f7',
        selectionBackground: '#2a3a55',
      },
    });

    this.fitAddon = new FitAddon();
    this.terminal.loadAddon(this.fitAddon);
    this.terminal.loadAddon(new WebLinksAddon());
    this.terminal.open(this.mountElement);

    requestAnimationFrame(() => {
      this.fitAddon.fit();
      const { cols, rows } = this.terminal;
      bus.resizeTerminal(this.id, cols, rows);
    });

    const resizeObserver = new ResizeObserver(() => {
      this.fitAddon.fit();
      const { cols, rows } = this.terminal;
      bus.resizeTerminal(this.id, cols, rows);
    });
    resizeObserver.observe(this.mountElement);

    this.terminal.onData((data) => {
      const bytes = new TextEncoder().encode(data);
      bus.sendTerminalInput(this.id, bytes);
    });
  }

  setupBusListeners() {
    bus.on('TerminalOutput', (payload) => {
      if (payload.workspace_id === this.id && this.running) {
        const bytes = new Uint8Array(payload.bytes);
        this.terminal.write(bytes);
      }
    });

    bus.on('WorkspaceExited', (payload) => {
      if (payload.workspace_id === this.id) {
        this.running = false;
        this.statusSpan.style.color = 'var(--danger)';
        this.statusSpan.innerHTML = '● exited';
        this.terminal.writeln('\r\n\x1b[31m[Process exited]\x1b[0m');
      }
    });

    bus.on('WorkspaceError', (payload) => {
      if (payload.workspace_id === this.id) {
        this.running = false;
        this.statusSpan.style.color = 'var(--danger)';
        this.statusSpan.innerHTML = '● error';
        this.terminal.writeln(`\r\n\x1b[31m[Error: ${payload.error}]\x1b[0m`);
      }
    });
  }

  destroy() {
    this.terminal.dispose();
    this.element.remove();
  }
}

class App {
  constructor() {
    this.currentView = 'welcome';
    this.currentSessionId = null;
    this.sessions = [];
    this.agentTypes = [];
    this.workspaces = new Map();
    this.paneInstances = new Map();

    this.setupDOMRefs();
    this.setupDOMListeners();
    this.setupBusListeners();
    bus.ready.then(() => this.requestInitialData());
  }

  setupDOMRefs() {
    this.sessionListEl = document.getElementById('session-list');
    this.sessionTitleEl = document.getElementById('session-title');
    this.sessionSep = document.getElementById('session-sep');
    this.sessionStatus = document.getElementById('session-status');
    this.agentCountEl = document.getElementById('agent-count');
    this.panesContainer = document.getElementById('panes-container');
    this.welcomeView = document.getElementById('view-welcome');
    this.sessionView = document.getElementById('view-session');
    this.createDialog = document.getElementById('create-dialog');
    this.sessionNameInput = document.getElementById('session-name');
    this.agentTypeSelect = document.getElementById('agent-type');
    this.homeFolderInput = document.getElementById('home-folder');
  }

  setupDOMListeners() {
    document.getElementById('new-session-btn').addEventListener('click', () => this.showCreateDialog());
    document.getElementById('welcome-new-btn').addEventListener('click', () => this.showCreateDialog());
    document.getElementById('cancel-create').addEventListener('click', () => this.hideCreateDialog());
    document.getElementById('confirm-create').addEventListener('click', () => this.handleCreateSession());
    document.getElementById('add-workspace-btn').addEventListener('click', () => this.handleAddWorkspace());
    document.getElementById('back-btn').addEventListener('click', () => this.handleBack());

    this.createDialog.addEventListener('click', (e) => {
      if (e.target === this.createDialog) this.hideCreateDialog();
    });
  }

  setupBusListeners() {
    bus.on('SessionList', (payload) => {
      this.sessions = payload.sessions || [];
      this.renderSessionList();
    });

    bus.on('AgentTypes', (payload) => {
      this.agentTypes = payload.types || [];
      if (!this.createDialog.classList.contains('show')) {
        this.populateAgentTypes();
      }
    });

    bus.on('SessionCreated', (payload) => {
      if (payload.session) {
        bus.attachToSession(payload.session.id);
      }
    });

    bus.on('SessionAttached', (payload) => {
      this.currentSessionId = payload.session.id;
      this.sessionTitleEl.textContent = payload.session.name;
      this.sessionSep.style.display = '';
      this.sessionStatus.style.display = '';
      this.agentCountEl.style.display = '';
      this.workspaces.clear();
      (payload.workspaces || []).forEach((ws) => this.workspaces.set(ws.id, ws));
      this.showSessionView();
      this.spawnInitialWorkspace();
    });

    bus.on('WorkspaceSpawned', (payload) => {
      if (payload.workspace) {
        this.workspaces.set(payload.workspace.id, payload.workspace);
        this.addPane(payload.workspace);
        this.updateAgentCount();
      }
    });

    bus.on('Error', (payload) => {
      console.error('Error:', payload.message);
    });

    bus.on('Status', (payload) => {
      console.log('Status:', payload.message);
    });
  }

  requestInitialData() {
    bus.listSessions();
    bus.listAgentTypes();
  }

  renderSessionList() {
    this.sessionListEl.innerHTML = '';
    if (this.sessions.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty';
      empty.textContent = 'No sessions yet';
      this.sessionListEl.appendChild(empty);
      return;
    }
    this.sessions.forEach((session) => {
      const item = document.createElement('div');
      item.className = 's-item';
      if (session.id === this.currentSessionId) item.classList.add('active');

      const dot = document.createElement('span');
      dot.className = 'dot idle';

      const meta = document.createElement('div');
      meta.className = 's-meta';

      const name = document.createElement('div');
      name.className = 's-name';
      name.textContent = session.name;

      const sub = document.createElement('div');
      sub.className = 's-sub';
      sub.textContent = session.home_folder || 'default';

      meta.appendChild(name);
      meta.appendChild(sub);

      item.appendChild(dot);
      item.appendChild(meta);
      item.addEventListener('click', () => bus.attachToSession(session.id));
      this.sessionListEl.appendChild(item);
    });
  }

  populateAgentTypes() {
    this.agentTypeSelect.innerHTML = '';
    if (this.agentTypes.length === 0) {
      const placeholder = document.createElement('option');
      placeholder.value = '';
      placeholder.textContent = 'Loading agents…';
      placeholder.disabled = true;
      this.agentTypeSelect.appendChild(placeholder);
      return;
    }
    this.agentTypes.forEach((at) => {
      const option = document.createElement('option');
      option.value = at.name;
      option.textContent = `${at.name} — ${at.description}`;
      this.agentTypeSelect.appendChild(option);
    });
  }

  showCreateDialog() {
    this.sessionNameInput.value = '';
    this.homeFolderInput.value = '';
    this.populateAgentTypes();
    this.createDialog.classList.add('show');
    this.sessionNameInput.focus();
  }

  hideCreateDialog() {
    this.createDialog.classList.remove('show');
  }

  handleCreateSession() {
    const name = this.sessionNameInput.value.trim() || 'Untitled';
    const agentType = this.agentTypeSelect.value;
    const homeFolder = this.homeFolderInput.value.trim() || null;
    if (!agentType) return;
    this.hideCreateDialog();
    bus.createSession(name, agentType, homeFolder);
  }

  showSessionView() {
    this.welcomeView.classList.remove('show');
    this.sessionView.classList.add('show');
    this.panesContainer.innerHTML = '';
    this.paneInstances.clear();
    this.currentView = 'session';
    this.renderSessionList();
  }

  showWelcomeView() {
    this.sessionView.classList.remove('show');
    this.welcomeView.classList.add('show');
    this.sessionSep.style.display = 'none';
    this.sessionStatus.style.display = 'none';
    this.agentCountEl.style.display = 'none';
    this.currentView = 'welcome';
    this.currentSessionId = null;
    this.renderSessionList();
    bus.listSessions();
  }

  handleBack() {
    this.paneInstances.forEach((p) => p.destroy());
    this.paneInstances.clear();
    this.workspaces.clear();
    if (this.currentSessionId) {
      bus.detachFromSession(this.currentSessionId);
    }
    this.showWelcomeView();
  }

  handleAddWorkspace() {
    if (!this.currentSessionId || this.agentTypes.length === 0) return;
    const agentType = this.agentTypes[0].name;
    bus.spawnWorkspace(this.currentSessionId, agentType, null);
  }

  spawnInitialWorkspace() {
    if (!this.currentSessionId || this.agentTypes.length === 0) return;
    const agentType = this.agentTypes[0].name;
    bus.spawnWorkspace(this.currentSessionId, agentType, null);
  }

  addPane(workspaceInfo) {
    const pane = new WorkspacePane(workspaceInfo, this.panesContainer);
    this.paneInstances.set(workspaceInfo.id, pane);
    pane.terminal.focus();
  }

  updateAgentCount() {
    const count = this.paneInstances.size;
    this.agentCountEl.textContent = `agent ${count}`;
  }
}

document.addEventListener('DOMContentLoaded', () => {
  window.app = new App();
});
