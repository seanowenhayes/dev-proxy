import { useState, useEffect } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';
import { Button } from './components/ui/Button';

type ProxyEvent = { event: 'started', data: string }
    | { event: 'connectionAccepted', data: string }
    | { event: 'connectionError', data: string }
    | { event: 'tunnel', data: { addr: string, fromClient: number, fromServer: number } };

const onEvent = new Channel<ProxyEvent>();

function App() {
    const [running, setRunning] = useState(false);
    const [messages, setMessages] = useState<string[]>([]);

    const start = async () => {
        await invoke('start_proxy', { onEvent });
        setRunning(true);
    };

    onEvent.onmessage = (message) => {
        switch (message.event) {
            case 'started':
                setRunning(true);
                setMessages((prev) => [...prev, `Proxy started at ${message.data}`]);
                break;
            case 'connectionAccepted':
                setMessages((prev) => [...prev, `Connection accepted from ${message.data}`]);
                break;
            case 'connectionError':
                setMessages((prev) => [...prev, `Connection error: ${message.data}`]);
                setRunning(false);
                break;
            case 'tunnel':
                setMessages((prev) => [...prev, `Tunnel established to ${message.data.addr} (client: ${message.data.fromClient} bytes, server: ${message.data.fromServer} bytes)`]);
                break;
            default: console.log('Unknown event', message);
        }
    };

    return (
        <div className="p-4">
            <h1 className="text-2xl font-bold mb-4">Proxy Control</h1>
            <p>Status: {running ? 'running' : 'stopped'}</p>
            <Button className="mt-2" onClick={running ? stop : start}>
                {running ? 'Stop' : 'Start'}
            </Button>
            <div className="mt-4">
                <h2 className="text-xl font-semibold mb-2">Messages:</h2>
                <ul className="list-disc list-inside">
                    {messages.map((msg, index) => (
                        <li key={index}>{msg}</li>
                    ))}
                </ul>
            </div>
        </div>
    );
}

export default App;
