import { useState, useEffect, useCallback, useRef } from 'react';
import Editor, { type OnMount } from '@monaco-editor/react';
import { Play, Loader2, BookOpen, ChevronRight, Terminal, Clock, CheckCircle, XCircle, Keyboard } from 'lucide-react';
import './App.css';

interface Example {
  id: string;
  name: string;
  category: string;
  description: string;
  code: string;
}

interface RunResult {
  success: boolean;
  stdout: string;
  stderr: string;
  duration_ms: number;
}

const API = import.meta.env.DEV ? 'http://localhost:9876' : '';
const IS_MAC = navigator.platform.toUpperCase().includes('MAC');
const MOD_KEY = IS_MAC ? '⌘' : 'Ctrl';

function App() {
  const [examples, setExamples] = useState<Example[]>([]);
  const [code, setCode] = useState('');
  const [selectedId, setSelectedId] = useState('');
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<RunResult | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [backendStatus, setBackendStatus] = useState<'checking' | 'online' | 'offline'>('checking');
  const runRef = useRef<() => void>(undefined);

  const runCode = useCallback(async () => {
    if (running || !code.trim()) return;
    setRunning(true);
    setResult(null);
    try {
      const res = await fetch(`${API}/api/run`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code, example_id: selectedId }),
      });
      const data: RunResult = await res.json();
      setResult(data);
    } catch (e) {
      setResult({
        success: false,
        stdout: '',
        stderr: `Connection error: ${e}`,
        duration_ms: 0,
      });
    }
    setRunning(false);
  }, [code, selectedId, running]);

  runRef.current = runCode;

  useEffect(() => {
    fetch(`${API}/api/health`)
      .then(() => setBackendStatus('online'))
      .catch(() => setBackendStatus('offline'));

    fetch(`${API}/api/examples`)
      .then(r => r.json())
      .then((data: Example[]) => {
        setExamples(data);
        if (data.length > 0) {
          setCode(data[0].code);
          setSelectedId(data[0].id);
        }
      })
      .catch(() => {
        setCode(`// Backend not reachable. Start it with:\n// cargo run --manifest-path playground/backend/Cargo.toml`);
      });
  }, []);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
        e.preventDefault();
        runRef.current?.();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);

  const handleEditorMount: OnMount = (editor, monaco) => {
    editor.addAction({
      id: 'run-code',
      label: 'Run Code',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
      run: () => runRef.current?.(),
    });
  };

  const selectExample = (ex: Example) => {
    setCode(ex.code);
    setSelectedId(ex.id);
    setResult(null);
  };

  const categories = [...new Set(examples.map(e => e.category))];

  return (
    <div className="app">
      <header className="header">
        <div className="header-left">
          <h1>⚡ ADK Playground</h1>
          <span className="subtitle">Rust Agent Development Kit</span>
          <span className={`status-dot ${backendStatus}`} title={`Backend: ${backendStatus}`} />
        </div>
        <div className="header-right">
          <span className="shortcut-hint">
            <Keyboard size={12} />
            {MOD_KEY}+Enter
          </span>
          <button
            className={`run-btn ${running ? 'running' : ''}`}
            onClick={runCode}
            disabled={running || !code.trim() || backendStatus === 'offline'}
          >
            {running ? <Loader2 size={16} className="spin" /> : <Play size={16} />}
            {running ? 'Compiling...' : 'Run'}
          </button>
        </div>
      </header>

      <div className="main">
        {sidebarOpen && (
          <aside className="sidebar">
            <div className="sidebar-header">
              <BookOpen size={16} />
              <span>Examples</span>
              <span className="example-count">{examples.length}</span>
            </div>
            {categories.map(cat => (
              <div key={cat} className="category">
                <div className="category-name">{cat}</div>
                {examples.filter(e => e.category === cat).map(ex => (
                  <button
                    key={ex.id}
                    className={`example-btn ${selectedId === ex.id ? 'active' : ''}`}
                    onClick={() => selectExample(ex)}
                  >
                    <ChevronRight size={14} />
                    <div>
                      <div className="example-name">{ex.name}</div>
                      <div className="example-desc">{ex.description}</div>
                    </div>
                  </button>
                ))}
              </div>
            ))}
          </aside>
        )}

        <div className="editor-area">
          <div className="editor-toolbar">
            <button className="toggle-sidebar" onClick={() => setSidebarOpen(!sidebarOpen)} title="Toggle examples">
              <BookOpen size={14} />
            </button>
            <span className="file-name">main.rs</span>
            <span className="file-tag">
              {examples.find(e => e.id === selectedId)?.name || 'Custom'}
            </span>
            {result && (
              <span className="duration">
                <Clock size={12} />
                {(result.duration_ms / 1000).toFixed(1)}s
              </span>
            )}
          </div>
          <div className="editor-container">
            <Editor
              height="100%"
              defaultLanguage="rust"
              theme="vs-dark"
              value={code}
              onChange={(v) => setCode(v || '')}
              onMount={handleEditorMount}
              options={{
                fontSize: 14,
                minimap: { enabled: false },
                lineNumbers: 'on',
                scrollBeyondLastLine: false,
                automaticLayout: true,
                tabSize: 4,
                wordWrap: 'on',
                padding: { top: 12 },
                renderLineHighlight: 'gutter',
                smoothScrolling: true,
                cursorBlinking: 'smooth',
                cursorSmoothCaretAnimation: 'on',
                bracketPairColorization: { enabled: true },
              }}
            />
          </div>
        </div>

        <div className="output-area">
          <div className="output-header">
            <Terminal size={14} />
            <span>Output</span>
            {result && (
              result.success
                ? <CheckCircle size={14} className="success-icon" />
                : <XCircle size={14} className="error-icon" />
            )}
            {result && (
              <button className="clear-btn" onClick={() => setResult(null)}>Clear</button>
            )}
          </div>
          <div className="output-content">
            {backendStatus === 'offline' && !running && !result && (
              <div className="output-placeholder offline">
                <XCircle size={20} />
                <div>
                  <div>Backend offline</div>
                  <code>cargo run --manifest-path playground/backend/Cargo.toml</code>
                </div>
              </div>
            )}
            {backendStatus !== 'offline' && !result && !running && (
              <div className="output-placeholder">
                Click <strong>Run</strong> or press <strong>{MOD_KEY}+Enter</strong>
              </div>
            )}
            {running && (
              <div className="output-placeholder running">
                <Loader2 size={20} className="spin" />
                <div>
                  <div>Compiling and running...</div>
                  <div className="output-sub">First builds take longer</div>
                </div>
              </div>
            )}
            {result && (
              <pre className={`output-text ${result.success ? '' : 'error'}`}>
                {result.stdout && <span className="stdout">{result.stdout}</span>}
                {result.stderr && <span className="stderr">{result.stderr}</span>}
                {!result.stdout && !result.stderr && (
                  <span className="stdout">(no output)</span>
                )}
              </pre>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
