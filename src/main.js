import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import '@xterm/xterm/css/xterm.css';

// Tauri API import (will be used for backend communication)
// import { invoke } from '@tauri-apps/api/core';

class Pane {
  constructor(id, container) {
    this.id = id;
    this.container = container;
    this.terminal = null;
    this.fitAddon = null;
    this.element = null;
    this.headerElement = null;
    
    this.createElement();
    this.initTerminal();
  }
  
  createElement() {
    this.element = document.createElement('div');
    this.element.className = 'pane';
    this.element.dataset.paneId = this.id;
    
    this.headerElement = document.createElement('div');
    this.headerElement.className = 'pane-header';
    this.headerElement.innerHTML = `
      <span>Pane ${this.id}</span>
      <span>●</span>
    `;
    
    const contentElement = document.createElement('div');
    contentElement.className = 'pane-content';
    
    this.element.appendChild(this.headerElement);
    this.element.appendChild(contentElement);
    this.container.appendChild(this.element);
    
    this.contentElement = contentElement;
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
      }
    });
    
    this.fitAddon = new FitAddon();
    this.terminal.loadAddon(this.fitAddon);
    this.terminal.loadAddon(new WebLinksAddon());
    
    this.terminal.open(this.contentElement);
    this.fitAddon.fit();
    
    // Handle resize
    const resizeObserver = new ResizeObserver(() => {
      this.fitAddon.fit();
      // TODO: Send resize event to backend
    });
    resizeObserver.observe(this.contentElement);
    
    // Handle input
    this.terminal.onData((data) => {
      // TODO: Send input to backend
      console.log(`Pane ${this.id} input:`, data);
    });
    
    // Show welcome message
    this.terminal.writeln('Ubiq — Harness Multiplexer v0.1.0');
    this.terminal.writeln('Press Enter to start a harness...');
    this.terminal.writeln('');
  }
  
  write(data) {
    this.terminal.write(data);
  }
  
  resize(cols, rows) {
    this.terminal.resize(cols, rows);
  }
  
  focus() {
    this.terminal.focus();
  }
  
  destroy() {
    this.terminal.dispose();
    this.element.remove();
  }
}

class App {
  constructor() {
    this.panes = new Map();
    this.nextPaneId = 1;
    this.container = document.getElementById('panes-container');
    
    this.setupEventListeners();
    this.addPane(); // Start with one pane
  }
  
  setupEventListeners() {
    document.getElementById('add-pane').addEventListener('click', () => {
      this.addPane();
    });
    
    document.getElementById('remove-pane').addEventListener('click', () => {
      this.removeLastPane();
    });
    
    // Handle window resize
    window.addEventListener('resize', () => {
      this.panes.forEach(pane => {
        pane.fitAddon.fit();
      });
    });
  }
  
  addPane() {
    const id = this.nextPaneId++;
    const pane = new Pane(id, this.container);
    this.panes.set(id, pane);
    
    // Focus the new pane
    pane.focus();
    
    // Update status
    this.updateStatus(`Added pane ${id}`);
    
    return pane;
  }
  
  removeLastPane() {
    if (this.panes.size === 0) return;
    
    const lastId = Array.from(this.panes.keys()).pop();
    const pane = this.panes.get(lastId);
    pane.destroy();
    this.panes.delete(lastId);
    
    this.updateStatus(`Removed pane ${lastId}`);
  }
  
  updateStatus(message) {
    const statusElement = document.querySelector('.status-bar span:first-child');
    statusElement.textContent = message;
    
    // Reset to "Ready" after 3 seconds
    setTimeout(() => {
      statusElement.textContent = 'Ready';
    }, 3000);
  }
  
  // TODO: Implement Tauri backend communication
  async spawnHarness(paneId, harness, args) {
    // This will be implemented with Tauri invoke
    console.log(`Spawning ${harness} in pane ${paneId}`);
  }
  
  async sendInput(paneId, data) {
    // This will be implemented with Tauri invoke
    console.log(`Sending input to pane ${paneId}`);
  }
  
  async resizePane(paneId, cols, rows) {
    // This will be implemented with Tauri invoke
    console.log(`Resizing pane ${paneId} to ${cols}x${rows}`);
  }
}

// Initialize the app when DOM is loaded
document.addEventListener('DOMContentLoaded', () => {
  window.app = new App();
});