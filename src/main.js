import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';
import { bus } from './bus.js';

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
    this.element.className = 'workspace-pane';
    this.element.dataset.workspaceId = this.id;

    const header = document.createElement('div');
    header.className = 'pane-header';

    const badge = document.createElement('span');
    badge.className = 'agent-badge';
    badge.textContent = this.agentType;

    this.statusDot = document.createElement('span');
    this.statusDot.className = 'status-dot running';

    header.appendChild(badge);
    header.appendChild(this.statusDot);

    this.contentElement = document.createElement('div');
    this.contentElement.className = 'pane-content';

    this.element.appendChild(header);
    this.element.appendChild(this.contentElement);
    container.appendChild(this.element);
  }

  initTerminal() {
    this.terminal = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: 'Menlo, Monaco, "Courier New", monospace',
      theme: {
        background: '#1e1e1e',
        foreground: '#d4d4d4',
        cursor: '#d4d4d4',
        selectionBackground: '#264f78',
      },
    });

    this.fitAddon = new FitAddon();
    this.terminal.loadAddon(this.fitAddon);
    this.terminal.loadAddon(new WebLinksAddon());
    this.terminal.open(this.contentElement);

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
    resizeObserver.observe(this.contentElement);

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
        this.statusDot.className = 'status-dot exited';
        this.terminal.writeln('\r\n\x1b[31m[Process exited]\x1b[0m');
      }
    });

    bus.on('WorkspaceError', (payload) => {
      if (payload.workspace_id === this.id) {
        this.running = false;
        this.statusDot.className = 'status-dot exited';
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
    this.sessionHeaderEl = document.getElementById('session-header');
    this.sessionTitleEl = document.getElementById('session-title');
    this.panesContainer = document.getElementById('panes-container');
    this.welcomeView = document.getElementById('welcome-view');
    this.createDialog = document.getElementById('create-dialog');
    this.sessionNameInput = document.getElementById('session-name');
    this.agentTypeSelect = document.getElementById('agent-type');
    this.homeFolderInput = document.getElementById('home-folder');
    this.statusText = document.getElementById('status-text');
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
      if (!this.createDialog.classList.contains('visible')) {
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
      this.workspaces.clear();
      (payload.workspaces || []).forEach((ws) => this.workspaces.set(ws.id, ws));
      this.showSessionView();
      this.spawnInitialWorkspace();
    });

    bus.on('WorkspaceSpawned', (payload) => {
      if (payload.workspace) {
        this.workspaces.set(payload.workspace.id, payload.workspace);
        this.addPane(payload.workspace);
      }
    });

    bus.on('Error', (payload) => {
      this.updateStatus(payload.message || 'Error occurred');
    });

    bus.on('Status', (payload) => {
      this.updateStatus(payload.message || '');
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
      empty.style.cssText = 'padding: 12px; font-size: 12px; color: #666;';
      empty.textContent = 'No sessions yet';
      this.sessionListEl.appendChild(empty);
      return;
    }
    this.sessions.forEach((session) => {
      const item = document.createElement('div');
      item.className = 'session-item';
      if (session.id === this.currentSessionId) item.classList.add('active');

      const name = document.createElement('div');
      name.className = 'session-name';
      name.textContent = session.name;

      const meta = document.createElement('div');
      meta.className = 'session-meta';
      meta.textContent = session.home_folder;

      item.appendChild(name);
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
      placeholder.textContent = 'Loading agents...';
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
    this.createDialog.classList.add('visible');
    this.sessionNameInput.focus();
  }

  hideCreateDialog() {
    this.createDialog.classList.remove('visible');
  }

  handleCreateSession() {
    const name = this.sessionNameInput.value.trim() || 'Untitled';
    const agentType = this.agentTypeSelect.value;
    const homeFolder = this.homeFolderInput.value.trim() || null;
    if (!agentType) {
      this.updateStatus('No agent type selected');
      return;
    }
    this.hideCreateDialog();
    bus.createSession(name, agentType, homeFolder);
  }

  showSessionView() {
    this.welcomeView.style.display = 'none';
    this.sessionHeaderEl.style.display = 'flex';
    this.panesContainer.style.display = 'flex';
    this.panesContainer.innerHTML = '';
    this.paneInstances.clear();
    this.currentView = 'session';
    this.renderSessionList();
  }

  showWelcomeView() {
    this.welcomeView.style.display = 'flex';
    this.sessionHeaderEl.style.display = 'none';
    this.panesContainer.style.display = 'none';
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

  updateStatus(msg) {
    this.statusText.textContent = msg;
    clearTimeout(this._statusTimeout);
    this._statusTimeout = setTimeout(() => {
      this.statusText.textContent = 'Ready';
    }, 5000);
  }
}

document.addEventListener('DOMContentLoaded', () => {
  window.app = new App();
});
