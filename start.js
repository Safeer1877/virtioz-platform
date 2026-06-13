const { spawn, execSync } = require('child_process');
const readline = require('readline');
const path = require('path');
const os = require('os');

const PORT_LIST = [3000, 8080, 9001, 9002];

function killPort(port) {
  try {
    if (os.platform() === 'win32') {
      let output = '';
      try {
        output = execSync(`netstat -ano | findstr :${port}`).toString();
      } catch (e) {
        return; // findstr returns error if no match
      }
      
      const lines = output.trim().split('\n');
      for (const line of lines) {
        if (!line) continue;
        const parts = line.trim().split(/\s+/);
        // Ensure exact match for the port in the local address (e.g., 0.0.0.0:8080) and that it's listening
        if (parts.length >= 5 && parts[1].endsWith(`:${port}`) && parts[3] === 'LISTENING') {
          const pid = parts[4];
          try {
            execSync(`taskkill /PID ${pid} /F`);
            console.log(`Killed stale process PID ${pid} on port ${port}`);
          } catch (e) {
            // Process may already be dead
          }
        }
      }
      
      // Verify port is free before returning
      try {
        const verifyOutput = execSync(`netstat -ano | findstr :${port}`).toString();
        const verifyLines = verifyOutput.trim().split('\n');
        for (const line of verifyLines) {
          if (!line) continue;
          const parts = line.trim().split(/\s+/);
          if (parts.length >= 5 && parts[1].endsWith(`:${port}`) && parts[3] === 'LISTENING') {
             console.warn(`Warning: Port ${port} may still be in use by PID ${parts[4]}.`);
          }
        }
      } catch (e) {
         // Good, findstr failed meaning nothing is listening
      }
    } else {
      try {
        execSync(`lsof -t -i:${port} | xargs kill -9 2>/dev/null`);
      } catch(e) {}
    }
  } catch (err) {
    console.error(`Error checking port ${port}:`, err);
  }
}

const processes = [];
let shuttingDown = false;

function cleanup() {
  if (shuttingDown) return;
  shuttingDown = true;
  console.log('\nCleaning up processes...');
  for (const p of processes) {
    try {
      if (os.platform() === 'win32') {
        execSync(`taskkill /pid ${p.pid} /T /F 2>nul`);
      } else {
        p.kill('SIGKILL');
      }
    } catch(e) {}
  }
  process.exit(0);
}

process.on('SIGINT', cleanup);
process.on('SIGTERM', cleanup);

const stages = [
  {
    name: 'Core Build',
    cmd: 'cargo',
    args: ['build'],
    cwd: 'core',
    readyMsg: 'Finished `dev` profile',
    prefix: '\x1b[35m[BUILD]\x1b[0m'
  },
  {
    name: 'Signaling Server',
    cmd: os.platform() === 'win32' ? '.\\target\\debug\\virtioz-signaling.exe' : './target/debug/virtioz-signaling',
    args: [],
    cwd: 'core',
    readyMsg: 'Starting Mesh Signaling Server',
    prefix: '\x1b[36m[SIGNALING]\x1b[0m'
  },
  {
    name: 'Swarm Agent',
    cmd: os.platform() === 'win32' ? '.\\target\\debug\\virtioz-agent.exe' : './target/debug/virtioz-agent',
    args: [],
    cwd: 'core',
    readyMsg: 'Swarm Agent starting',
    prefix: '\x1b[32m[AGENT]\x1b[0m'
  },
  {
    name: 'Core Router',
    cmd: os.platform() === 'win32' ? '.\\target\\debug\\virtioz-router.exe' : './target/debug/virtioz-router',
    args: [],
    cwd: 'core',
    readyMsg: 'Dashboard broadcaster listening',
    prefix: '\x1b[33m[ROUTER]\x1b[0m'
  },
  {
    name: 'Shell Webpack',
    cmd: os.platform() === 'win32' ? 'npm.cmd' : 'npm',
    args: ['start'],
    cwd: 'shell',
    readyMsg: 'Project is running at',
    prefix: '\x1b[34m[SHELL]\x1b[0m',
    postReadyDelay: 3000
  },
  {
    name: 'Shell Electron',
    cmd: os.platform() === 'win32' ? 'npm.cmd' : 'npm',
    args: ['run', 'electron:start'],
    cwd: 'shell',
    readyMsg: null,
    prefix: '\x1b[34m[SHELL]\x1b[0m'
  }
];

async function startService(stage) {
  return new Promise((resolve, reject) => {
    console.log(`Starting ${stage.name}...`);
    
    const p = spawn(stage.cmd, stage.args, {
      cwd: path.join(__dirname, stage.cwd),
      env: stage.env || process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
      shell: os.platform() === 'win32'
    });
    
    processes.push(p);

    const rlOut = readline.createInterface({ input: p.stdout });
    const rlErr = readline.createInterface({ input: p.stderr });

    let isReady = false;

    const timeout = setTimeout(() => {
      if (!isReady && stage.readyMsg) {
        reject(new Error(`Timeout waiting for ${stage.name} to be ready.`));
      }
    }, 600000);

    rlOut.on('line', (line) => {
      console.log(`${stage.prefix} ${line}`);
      if (!isReady && stage.readyMsg && line.toLowerCase().includes(stage.readyMsg.toLowerCase())) {
        isReady = true;
        clearTimeout(timeout);
        resolve();
      }
    });

    rlErr.on('line', (line) => {
      console.log(`${stage.prefix} ${line}`);
      if (!isReady && stage.readyMsg && line.toLowerCase().includes(stage.readyMsg.toLowerCase())) {
        isReady = true;
        clearTimeout(timeout);
        resolve();
      }
    });

    p.on('error', (err) => {
      console.log(`${stage.prefix} Error: ${err.message}`);
      if (!isReady) {
        clearTimeout(timeout);
        reject(err);
      }
    });

    p.on('exit', (code) => {
      if (!isReady && stage.readyMsg) {
        clearTimeout(timeout);
        reject(new Error(`${stage.name} exited with code ${code} before becoming ready.`));
      }
    });

    if (!stage.readyMsg) {
      isReady = true;
      clearTimeout(timeout);
      resolve();
    }
  });
}

async function run() {
  console.log('Killing stale processes on ports: ' + PORT_LIST.join(', '));
  for (const port of PORT_LIST) {
    killPort(port);
  }
  console.log('Ports cleared.\n');

  for (let i = 0; i < stages.length; i++) {
    const stage = stages[i];
    try {
      await startService(stage);
      if (stage.postReadyDelay) {
        console.log(`${stage.prefix} Waiting ${stage.postReadyDelay/1000} seconds before next service...`);
        await new Promise(r => setTimeout(r, stage.postReadyDelay));
      }
    } catch (err) {
      console.error(`\x1b[31mFailed to start ${stage.name}:\x1b[0m`, err);
      cleanup();
      return;
    }
  }
  
  console.log('\n\x1b[32m=================================\x1b[0m');
  console.log('\x1b[32m       VIRTIOZ IS RUNNING        \x1b[0m');
  console.log('\x1b[32m=================================\x1b[0m');
  stages.forEach(s => {
    console.log(`${s.prefix} ${s.name} Service is UP`);
  });
}

run();
