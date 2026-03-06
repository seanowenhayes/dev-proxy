import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from './components/ui/Button';

function App() {
    const [running, setRunning] = useState(false);

    const checkStatus = async () => {
        const status: boolean = await invoke('status_proxy');
        setRunning(status);
    };

    const start = async () => {
        await invoke('start_proxy');
        checkStatus();
    };
    const stop = async () => {
        await invoke('stop_proxy');
        checkStatus();
    };

    useEffect(() => {
        checkStatus();
    }, []);

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
