const WebSocket = require('ws');

const ws = new WebSocket('ws://127.0.0.1:9002');

ws.on('open', () => {
    console.log("Connected to router");
    ws.send(JSON.stringify({
        type: 'auth',
        token: 'virtioz_local_ipc_admin'
    }));
});

ws.on('message', (data) => {
    const msg = JSON.parse(data.toString());
    console.log("RCV:", msg);
    
    if (msg.type === 'status' && msg.message === 'IPC Authenticated') {
        setTimeout(() => {
            console.log("Sending task...");
            ws.send(JSON.stringify({
                type: 'task',
                payload: 'echo "Hello GitHub, the Virtioz Swarm is ALIVE!"'
            }));
        }, 1000);
    }
});

ws.on('close', () => {
    console.log("Disconnected.");
});
