import React, { useState, useEffect, useRef } from 'react';
import { Terminal } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';
import 'xterm/css/xterm.css';
import './workspace.css';
import logo from '../logo.png';

const SovereignDashboard = ({ session, profile, handleSignOut }) => {
  const [nodes, setNodes] = useState([]);
  const [commandInput, setCommandInput] = useState('');
  const [activeTaskId, setActiveTaskId] = useState(null);
  const activeTaskIdRef = useRef(null);
  
  const coreWs = useRef(null);
  const terminalRef = useRef(null);
  const xterm = useRef(null);
  const fitAddon = useRef(null);

  useEffect(() => {
    // Initialize xterm.js
    xterm.current = new Terminal({
      theme: {
        background: 'transparent',
        foreground: '#f8fafc', // Crisp White
        cursor: '#f472b6', // Vibrant Magenta
        selectionBackground: 'rgba(244, 114, 182, 0.25)',
        black: '#0f172a',
        red: '#f43f5e',
        green: '#a78bfa', // Soft Purple
        yellow: '#fbbf24',
        blue: '#22d3ee', // Bright Cyan
        magenta: '#f472b6', // Magenta
        cyan: '#06b6d4',
        white: '#ffffff',
        brightBlack: '#475569',
        brightRed: '#fb7185',
        brightGreen: '#c084fc',
        brightYellow: '#fcd34d',
        brightBlue: '#67e8f9',
        brightMagenta: '#f9a8d4',
        brightCyan: '#67e8f9',
        brightWhite: '#f8fafc'
      },
      fontFamily: '"JetBrains Mono", "Consolas", "Courier New", monospace',
      fontSize: 15,
      fontWeight: '500',
      cursorBlink: true,
      convertEol: true
    });
    
    fitAddon.current = new FitAddon();
    xterm.current.loadAddon(fitAddon.current);
    
    if (terminalRef.current) {
      xterm.current.open(terminalRef.current);
      fitAddon.current.fit();
    }
    
    xterm.current.writeln('\x1b[1;35mVirtioz Swarm Orchestrator [Version 1.0.0]\x1b[0m');
    xterm.current.writeln('\x1b[35m(c) Virtioz Platform. All rights reserved.\x1b[0m\r\n');
    xterm.current.writeln('\x1b[34mEstablishing encrypted connection to local Swarm Router...\x1b[0m');

    // Connect to local Core Daemon
    coreWs.current = new WebSocket('ws://127.0.0.1:9002');
    
    coreWs.current.onopen = () => {
      // Send IPC Auth Token
      coreWs.current.send(JSON.stringify({
        type: 'auth',
        token: 'virtioz_local_ipc_admin'
      }));
    };

    coreWs.current.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);
        
        if (data.type === 'telemetry') {
          const activeCount = data.active_nodes || 0;
          const generatedNodes = [];
          for (let i = 0; i < activeCount; i++) {
            generatedNodes.push({
              id: `swarm_node_${i}`,
              name: `Swarm Node ${i + 1}`,
              status: 'active',
              icon: '🖥️'
            });
          }
          setNodes(generatedNodes);
        } else if (data.type === 'status') {
          xterm.current.writeln(`\x1b[34m[Router] ${data.message}\x1b[0m`);
          // Capture task ID for stdin routing
          if (data.message && data.message.startsWith("Task queued: ")) {
              const taskId = data.message.split("Task queued: ")[1];
              setActiveTaskId(taskId);
              activeTaskIdRef.current = taskId;
          }
        } else if (data.type === 'stream_chunk') {
          // Print raw data to xterm
          xterm.current.write(data.data);
        } else if (data.type === 'result') {
          const peer = data.source_peer ? `[${data.source_peer}]` : "[Agent]";
          if (data.error) {
            xterm.current.writeln(`\r\n\x1b[31m${peer} [ERROR] ${data.error}\x1b[0m`);
          } else {
            xterm.current.writeln(`\r\n\x1b[35m${peer} Process exited with code ${data.exit_code || 0}\x1b[0m`);
          }
          setActiveTaskId(null);
        }
      } catch (err) {
        console.error("Failed to parse core telemetry:", err);
      }
    };

    coreWs.current.onclose = () => {
      xterm.current.writeln('\r\n\x1b[31m[System] Disconnected from Rust Router.\x1b[0m');
    };

    // Listen for keystrokes in xterm and send as STDIN
    xterm.current.onData(data => {
      if (coreWs.current && coreWs.current.readyState === WebSocket.OPEN && activeTaskIdRef.current) {
        // Local echo so the user sees what they type
        xterm.current.write(data);
        
        // Send stdin chunk to backend router
        coreWs.current.send(JSON.stringify({
          type: 'stdin',
          task_id: activeTaskIdRef.current,
          data: data
        }));
      }
    });

    const handleResize = () => fitAddon.current.fit();
    window.addEventListener('resize', handleResize);
    
    return () => {
      window.removeEventListener('resize', handleResize);
      if (coreWs.current) coreWs.current.close();
      if (xterm.current) xterm.current.dispose();
    };
  }, []); // Empty dependency array ensures xterm and websocket are only mounted once

  const handleCommandSubmit = async (e) => {
    e.preventDefault();
    if (!commandInput.trim()) return;
    
    xterm.current.writeln(`\r\n> ${commandInput}`);
    
    if (coreWs.current && coreWs.current.readyState === WebSocket.OPEN) {
      coreWs.current.send(JSON.stringify({
        type: 'task',
        payload: commandInput
      }));
    } else {
      xterm.current.writeln('\x1b[31m[System] Cannot dispatch task. Router is offline.\x1b[0m');
    }
    
    setCommandInput('');
  };

  return (
    <div className="sovereign-dashboard">
      <div className="dashboard-sidebar">
        <div className="sidebar-header">
          <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
            <img src={logo} alt="Virtioz Logo" style={{ width: '32px', filter: 'drop-shadow(0 2px 8px rgba(244, 114, 182, 0.4))' }} />
            <h3>Virtioz Shell</h3>
          </div>
          <span className="badge private">Private Network</span>
        </div>
        
        <div className="devices-list">
          <div className="device-card me">
            <div className="device-icon">💻</div>
            <div className="device-info">
              <div className="device-name">This Device</div>
              <div className="device-status">
                <div className="status-dot active"></div>
                Primary Controller
              </div>
            </div>
          </div>
          
          {nodes.map((node) => (
            <div className="device-card active" key={node.id}>
              <div className="device-icon">{node.icon}</div>
              <div className="device-info">
                <div className="device-name">{node.name}</div>
                <div className="device-status">
                  <div className={`status-dot ${node.status === 'active' ? 'active' : ''}`}></div>
                  {node.status === 'active' ? 'Online & Ready' : 'Idle'}
                </div>
              </div>
            </div>
          ))}
        </div>
        
        <div className="sidebar-footer">
          <p>Install Virtioz on other devices to expand your cluster.</p>
          <button onClick={handleSignOut} className="sidebar-signout">Sign Out</button>
        </div>
      </div>
      
      <div className="dashboard-main">
        <div className="main-header">
          <h2>DISTRIBUTED TERMINAL</h2>
          <div className="header-badges">
            <span className="badge secure"><span className="icon">🔒</span> End-to-End Encrypted</span>
          </div>
        </div>
        
        <div className="terminal-container">
          <div className="terminal-logs" ref={terminalRef} style={{ width: '100%', flex: 1, overflow: 'hidden' }}>
            {/* xterm.js will mount here */}
          </div>
          
          <div className="terminal-input-area">
            <form onSubmit={handleCommandSubmit}>
              <span className="terminal-prompt">{'>'}</span>
              <input
                type="text"
                value={commandInput}
                onChange={(e) => setCommandInput(e.target.value)}
                placeholder="Dispatch command to Swarm..."
                spellCheck="false"
                autoComplete="off"
              />
              <button type="submit" disabled={!commandInput.trim()}>EXECUTE</button>
            </form>
          </div>
        </div>
        
        <div className="status-bar">
          <div className="status-item">
            <span className="label">ROUTER:</span>
            <span className="value connected">CONNECTED</span>
          </div>
          <div className="status-item">
            <span className="label">ACTIVE NODES:</span>
            <span className="value">{nodes.length}</span>
          </div>
          <div className="status-item">
            <span className="label">PARALLEL THREADS:</span>
            <span className="value">{nodes.length}</span>
          </div>
          <div className="status-item">
            <span className="label">NETWORK:</span>
            <span className="value">{nodes.length > 0 ? 'MESH SYNCHRONIZED' : 'WAITING FOR PEERS'}</span>
          </div>
        </div>
      </div>
    </div>
  );
};

export default SovereignDashboard;
