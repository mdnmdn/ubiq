import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

class Bus {
    constructor() {
        this.listeners = new Map();
        this.ready = this._setupListener();
    }

    async _setupListener() {
        const unlisten = await listen('bus:message', (event) => {
            const msg = JSON.parse(event.payload);
            const handlers = this.listeners.get(msg.type);
            if (handlers) {
                handlers.forEach((h) => h(msg.payload));
            }
            const all = this.listeners.get('*');
            if (all) {
                all.forEach((h) => h(msg));
            }
        });
        this._unlisten = unlisten;
    }

    async send(message) {
        const json = JSON.stringify(message);
        await invoke('bus_command', { message: json });
    }

    on(type, handler) {
        if (!this.listeners.has(type)) {
            this.listeners.set(type, new Set());
        }
        this.listeners.get(type).add(handler);
        return () => this.listeners.get(type).delete(handler);
    }

    async listSessions() {
        await this.send({ type: 'ListSessions' });
    }

    async createSession(name, agentType, homeFolder) {
        await this.send({
            type: 'CreateSession',
            payload: { name, agent_type: agentType, home_folder: homeFolder || null },
        });
    }

    async listAgentTypes() {
        await this.send({ type: 'ListAgentTypes' });
    }

    async attachToSession(sessionId) {
        await this.send({
            type: 'AttachToSession',
            payload: { session_id: sessionId },
        });
    }

    async detachFromSession(sessionId) {
        await this.send({
            type: 'DetachFromSession',
            payload: { session_id: sessionId },
        });
    }

    async spawnWorkspace(sessionId, agentType, folder) {
        await this.send({
            type: 'SpawnWorkspace',
            payload: { session_id: sessionId, agent_type: agentType, folder: folder || null },
        });
    }

    async sendTerminalInput(workspaceId, bytes) {
        await this.send({
            type: 'TerminalInput',
            payload: { workspace_id: workspaceId, bytes: Array.from(bytes) },
        });
    }

    async resizeTerminal(workspaceId, cols, rows) {
        await this.send({
            type: 'TerminalResize',
            payload: { workspace_id: workspaceId, cols, rows },
        });
    }

    async reconnectWorkspace(workspaceId, cols, rows) {
        await this.send({
            type: 'ReconnectWorkspace',
            payload: { workspace_id: workspaceId, cols, rows },
        });
    }
}

export const bus = new Bus();
