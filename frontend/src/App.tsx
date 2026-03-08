import { useState, useEffect } from 'react';
import { invoke, Channel } from '@tauri-apps/api/core';
import { Button } from './components/ui/Button';

type ProxyEvent = { event: 'started', data: { addr: string } } | { event: 'stopped' } | { event: 'error', data: { message: string } };
const onEvent = new Channel<ProxyEvent>();

function App() {
    const [running, setRunning] = useState(false);
    const [messages, setMessages] = useState<string[]>([]);

    const start = async () => {
        await invoke('start_proxy', { onEvent });
        setRunning(true);
    };

    useEffect(() => {
        onEvent.onmessage = (message) => {
            console.log(`got proxy event ${message.event}`);
            if (message.event === 'started') {
                setMessages((prev) => [...prev, `Proxy started at ${message.data.addr}`]);
            } else if (message.event === 'stopped') {
                setMessages((prev) => [...prev, 'Proxy stopped']);
                setRunning(false);
            } else if (message.event === 'error') {
                setMessages((prev) => [...prev, `Error: ${message.data.message}`]);
            }
        };
    }, []);

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
