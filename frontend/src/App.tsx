import { useState, useEffect } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';
import { Button } from './components/ui/Button';

type ProxyEvent = { event: 'started', data: { addr: string } } | { event: 'stopped' } | { event: 'error', data: { message: string } };
const onEvent = new Channel<ProxyEvent>();
onEvent.onmessage = (message) => {
    console.log(`got download event ${message.event}`);
};

function App() {
    const [running, setRunning] = useState(false);

    const start = async () => {
        await invoke('start_proxy', { onEvent });
        setRunning(true);
    };

    return (
        <div className="p-4">
            <h1 className="text-2xl font-bold mb-4">Proxy Control</h1>
            <p>Status: {running ? 'running' : 'stopped'}</p>
            <Button className="mt-2" onClick={running ? stop : start}>
                {running ? 'Stop' : 'Start'}
            </Button>
        </div>
    );
}

export default App;
