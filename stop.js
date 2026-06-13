const { execSync } = require('child_process');
const os = require('os');

const PORT_LIST = [8080, 9001, 9002];

function killPort(port) {
  try {
    if (os.platform() === 'win32') {
      let output = '';
      try {
        output = execSync(`netstat -ano | findstr :${port}`).toString();
      } catch (e) {
        return;
      }
      
      const lines = output.trim().split('\n');
      for (const line of lines) {
        if (!line) continue;
        const parts = line.trim().split(/\s+/);
        if (parts.length >= 5 && parts[1].endsWith(`:${port}`) && parts[3] === 'LISTENING') {
          const pid = parts[4];
          try {
            execSync(`taskkill /PID ${pid} /F`);
            console.log(`Killed process PID ${pid} on port ${port}`);
          } catch (e) {
          }
        }
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

console.log('Stopping Virtioz processes...');
for (const port of PORT_LIST) {
  killPort(port);
}
console.log('Done.');
