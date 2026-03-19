import { useState, useEffect, useCallback, useRef } from 'react';
import Editor, { type OnMount } from '@monaco-editor/react';
import { Play, Loader2, BookOpen, ChevronRight, ChevronDown, Terminal, Clock, CheckCircle, XCircle, Keyboard, Lock, Globe, Activity, Cpu, Wrench, MessageSquare, AlertTriangle, Bot, Github, Star, Zap, DollarSign, Hash } from 'lucide-react';
import './App.css';

interface Example {
  id: string;
  name: string;
  category: string;
  description: string;
  code: string;
}

interface TraceEvent {
  timestamp_ms: number;
  level: string;
  name: string;
  message: string;
  agent?: string;
  tool?: string;
  detail?: string;
  kind: string;
  target?: string;
}

interface RunSummary {
  compile_ms: number;
  run_ms: number;
  model?: string;
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  cost_estimate?: number;
}

interface RunResult {
  success: boolean;
  stdout: string;
  stderr: string;
  duration_ms: number;
  traces?: TraceEvent[];
  summary?: RunSummary;
}

interface ServerInfo {
  mode: 'public' | 'local';
  version: string;
  custom_code_enabled: boolean;
}

const API = import.meta.env.DEV ? 'http://localhost:9876' : '';
const IS_MAC = navigator.platform.toUpperCase().includes('MAC');
const MOD_KEY = IS_MAC ? '⌘' : 'Ctrl';

const TRACE_ICONS: Record<string, typeof Activity> = {
  agent: Bot,
  llm: Cpu,
  tool_call: Wrench,
  tool_result: CheckCircle,
  tool_error: AlertTriangle,
  info: MessageSquare,
  warn: AlertTriangle,
};

function TraceIcon({ kind }: { kind: string }) {
  const Icon = TRACE_ICONS[kind] || Activity;
  return <Icon size={12} />;
}

/** Group flat trace events into a tree: agent → llm/tool children */
interface TraceNode {
  event: TraceEvent;
  children: TraceNode[];
}

function buildTraceTree(events: TraceEvent[]): TraceNode[] {
  const roots: TraceNode[] = [];
  let currentAgent: TraceNode | null = null;

  for (const evt of events) {
    const node: TraceNode = { event: evt, children: [] };
    if (evt.kind === 'agent') {
      roots.push(node);
      currentAgent = node;
    } else if (currentAgent) {
      currentAgent.children.push(node);
    } else {
      roots.push(node);
    }
  }
  return roots;
}

function formatMs(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

function TraceDetail({ event, duration }: { event: TraceEvent; duration?: number }) {
  return (
    <div className="trace-detail-panel">
      <div className="trace-detail-grid">
        <span className="trace-detail-key">Kind</span>
        <span className={`trace-detail-val trace-kind-tag trace-kind-tag-${event.kind}`}>{event.kind}</span>
        <span className="trace-detail-key">Level</span>
        <span className="trace-detail-val">{event.level.toUpperCase()}</span>
        <span className="trace-detail-key">Time</span>
        <span className="trace-detail-val">{formatMs(event.timestamp_ms)}</span>
        {duration !== undefined && duration > 0 && (
          <>
            <span className="trace-detail-key">Duration</span>
            <span className="trace-detail-val trace-duration-val">{formatMs(duration)}</span>
          </>
        )}
        {event.agent && (
          <>
            <span className="trace-detail-key">Agent</span>
            <span className="trace-detail-val">{event.agent}</span>
          </>
        )}
        {event.tool && (
          <>
            <span className="trace-detail-key">Tool</span>
            <span className="trace-detail-val">{event.tool}</span>
          </>
        )}
        {event.target && (
          <>
            <span className="trace-detail-key">Target</span>
            <span className="trace-detail-val trace-target-val">{event.target}</span>
          </>
        )}
        <span className="trace-detail-key">Span</span>
        <span className="trace-detail-val">{event.name || '—'}</span>
      </div>
      {event.message && (
        <div className="trace-detail-section">
          <div className="trace-detail-section-label">Message</div>
          <pre className="trace-detail-pre">{event.message}</pre>
        </div>
      )}
      {event.detail && (
        <div className="trace-detail-section">
          <div className="trace-detail-section-label">Data</div>
          <pre className="trace-detail-pre">{tryFormatJson(event.detail)}</pre>
        </div>
      )}
    </div>
  );
}

function tryFormatJson(s: string): string {
  try {
    const parsed = JSON.parse(s);
    return JSON.stringify(parsed, null, 2);
  } catch {
    return s;
  }
}

function TraceTree({ traces }: { traces: TraceEvent[] }) {
  const tree = buildTraceTree(traces);
  const [expanded, setExpanded] = useState<Record<number, boolean>>({});
  const [selected, setSelected] = useState<string | null>(null);

  const toggleExpand = (i: number) => setExpanded(prev => ({ ...prev, [i]: !prev[i] }));
  const toggleSelect = (key: string) => setSelected(prev => prev === key ? null : key);

  if (tree.length === 0) {
    return <div className="trace-empty">No trace data captured</div>;
  }

  return (
    <div className="trace-tree">
      {tree.map((node, i) => {
        const isExpanded = expanded[i] ?? true;
        const hasChildren = node.children.length > 0;
        const nodeKey = `root-${i}`;
        const isSelected = selected === nodeKey;

        // Compute agent duration: time from first to last child (or own timestamp)
        const agentDuration = hasChildren
          ? node.children[node.children.length - 1].event.timestamp_ms - node.event.timestamp_ms
          : undefined;

        return (
          <div key={i} className="trace-node">
            <div
              className={`trace-row trace-kind-${node.event.kind} ${isSelected ? 'trace-row-selected' : ''}`}
              onClick={(e) => {
                if (hasChildren && !e.shiftKey) {
                  toggleExpand(i);
                }
                toggleSelect(nodeKey);
              }}
              style={{ cursor: 'pointer' }}
            >
              {hasChildren ? (
                isExpanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />
              ) : <span className="trace-spacer" />}
              <TraceIcon kind={node.event.kind} />
              <span className="trace-label">{node.event.agent || node.event.name}</span>
              <span className="trace-msg">{node.event.message}</span>
              {agentDuration !== undefined && agentDuration > 0 && (
                <span className="trace-timing">{formatMs(agentDuration)}</span>
              )}
              {hasChildren && <span className="trace-badge">{node.children.length}</span>}
            </div>
            {isSelected && (
              <TraceDetail event={node.event} duration={agentDuration} />
            )}
            {hasChildren && isExpanded && (
              <div className="trace-children">
                {node.children.map((child, j) => {
                  const childKey = `child-${i}-${j}`;
                  const isChildSelected = selected === childKey;
                  // Duration between consecutive children
                  const prevTs = j > 0 ? node.children[j - 1].event.timestamp_ms : node.event.timestamp_ms;
                  const childDuration = child.event.timestamp_ms - prevTs;

                  return (
                    <div key={j}>
                      <div
                        className={`trace-row trace-child trace-kind-${child.event.kind} ${isChildSelected ? 'trace-row-selected' : ''}`}
                        onClick={() => toggleSelect(childKey)}
                        style={{ cursor: 'pointer' }}
                      >
                        <span className="trace-spacer" />
                        <TraceIcon kind={child.event.kind} />
                        <span className="trace-label">
                          {child.event.tool || child.event.name}
                        </span>
                        <span className="trace-msg">{child.event.message}</span>
                        {child.event.detail && (
                          <span className="trace-detail-inline">{child.event.detail}</span>
                        )}
                        {childDuration > 0 && (
                          <span className="trace-timing">{formatMs(childDuration)}</span>
                        )}
                      </div>
                      {isChildSelected && (
                        <TraceDetail event={child.event} duration={childDuration > 0 ? childDuration : undefined} />
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function SummaryBar({ summary, success }: { summary: RunSummary; success: boolean }) {
  const fmtMs = (ms: number) => ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
  const fmtTokens = (n: number) => n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`;
  const fmtCost = (c: number) => c < 0.01 ? `<$0.01` : `$${c.toFixed(4)}`;

  return (
    <div className={`summary-bar ${success ? 'success' : 'error'}`}>
      <div className="summary-item">
        <Zap size={11} />
        <span className="summary-label">Compile</span>
        <span className="summary-value">{fmtMs(summary.compile_ms)}</span>
      </div>
      <div className="summary-item">
        <Play size={11} />
        <span className="summary-label">Run</span>
        <span className="summary-value">{fmtMs(summary.run_ms)}</span>
      </div>
      {summary.model && (
        <div className="summary-item">
          <Cpu size={11} />
          <span className="summary-value summary-model">{summary.model}</span>
        </div>
      )}
      {summary.input_tokens != null && (
        <div className="summary-item">
          <Hash size={11} />
          <span className="summary-label">In</span>
          <span className="summary-value">{fmtTokens(summary.input_tokens)}</span>
          {summary.output_tokens != null && (
            <>
              <span className="summary-sep">/</span>
              <span className="summary-label">Out</span>
              <span className="summary-value">{fmtTokens(summary.output_tokens)}</span>
            </>
          )}
        </div>
      )}
      {summary.cost_estimate != null && (
        <div className="summary-item summary-cost">
          <DollarSign size={11} />
          <span className="summary-value">{fmtCost(summary.cost_estimate)}</span>
        </div>
      )}
    </div>
  );
}

function App() {
  const [examples, setExamples] = useState<Example[]>([]);
  const [code, setCode] = useState('');
  const [selectedId, setSelectedId] = useState('');
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<RunResult | null>(null);
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [backendStatus, setBackendStatus] = useState<'checking' | 'online' | 'offline'>('checking');
  const [serverMode, setServerMode] = useState<ServerInfo | null>(null);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [activeTab, setActiveTab] = useState<'output' | 'trace'>('output');
  const runRef = useRef<() => void>(undefined);

  const isPublic = serverMode?.mode === 'public';

  const runCode = useCallback(async () => {
    if (running || !code.trim()) return;
    if (isPublic && !selectedId) return;
    setRunning(true);
    setResult(null);
    setActiveTab('output');
    try {
      const res = await fetch(`${API}/api/run`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code, example_id: selectedId || 'custom' }),
      });
      const data: RunResult = await res.json();
      setResult(data);
    } catch (e) {
      setResult({ success: false, stdout: '', stderr: `Connection error: ${e}`, duration_ms: 0 });
    }
    setRunning(false);
  }, [code, selectedId, running, isPublic]);

  runRef.current = runCode;

  useEffect(() => {
    fetch(`${API}/api/health`)
      .then(() => setBackendStatus('online'))
      .catch(() => setBackendStatus('offline'));
    fetch(`${API}/api/info`)
      .then(r => r.json())
      .then((info: ServerInfo) => setServerMode(info))
      .catch(() => {});
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

  const toggleCategory = (cat: string) => {
    setCollapsed(prev => ({ ...prev, [cat]: !prev[cat] }));
  };

  const categories = [...new Set(examples.map(e => e.category))];
  const traceCount = result?.traces?.length ?? 0;

  return (
    <div className="app">
      <header className="header">
        <div className="header-left">
          <h1>⚡ ADK Playground</h1>
          <span className="subtitle">Rust Agent Development Kit</span>
          <span className={`status-dot ${backendStatus}`} title={`Backend: ${backendStatus}`} />
          {serverMode && (
            <span className={`mode-badge ${serverMode.mode}`} title={
              isPublic ? 'Public mode — only registered examples can run' : 'Local mode — custom code enabled'
            }>
              {isPublic ? <Lock size={10} /> : <Globe size={10} />}
              {serverMode.mode}
            </span>
          )}
        </div>
        <div className="header-right">
          <a href="https://github.com/zavora-ai/adk-rust" target="_blank" rel="noopener noreferrer" className="github-star-link">
            <Github size={15} />
            <Star size={12} className="star-icon" />
            Star on GitHub
          </a>
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
            {categories.map(cat => {
              const catExamples = examples.filter(e => e.category === cat);
              const isCollapsed = collapsed[cat] ?? false;
              return (
                <div key={cat} className="category">
                  <button className="category-toggle" onClick={() => toggleCategory(cat)}>
                    {isCollapsed ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
                    <span className="category-name">{cat}</span>
                    <span className="category-count">{catExamples.length}</span>
                  </button>
                  {!isCollapsed && catExamples.map(ex => (
                    <button
                      key={ex.id}
                      className={`example-btn ${selectedId === ex.id ? 'active' : ''}`}
                      onClick={() => selectExample(ex)}
                    >
                      <div>
                        <div className="example-name">{ex.name}</div>
                        <div className="example-desc">{ex.description}</div>
                      </div>
                    </button>
                  ))}
                </div>
              );
            })}
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
              onChange={(v) => { if (!isPublic) setCode(v || ''); }}
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
                readOnly: isPublic,
              }}
            />
          </div>
        </div>

        <div className="output-area">
          <div className="output-tabs">
            <button
              className={`output-tab ${activeTab === 'output' ? 'active' : ''}`}
              onClick={() => setActiveTab('output')}
            >
              <Terminal size={13} />
              Output
              {result && (
                result.success
                  ? <CheckCircle size={12} className="success-icon" />
                  : <XCircle size={12} className="error-icon" />
              )}
            </button>
            <button
              className={`output-tab ${activeTab === 'trace' ? 'active' : ''}`}
              onClick={() => setActiveTab('trace')}
            >
              <Activity size={13} />
              Trace
              {traceCount > 0 && <span className="trace-count">{traceCount}</span>}
            </button>
            {result && (
              <button className="clear-btn" onClick={() => setResult(null)}>Clear</button>
            )}
          </div>

          <div className="output-content">
            {result?.summary && (
              <SummaryBar summary={result.summary} success={result.success} />
            )}
            {activeTab === 'output' && (
              <>
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
              </>
            )}
            {activeTab === 'trace' && (
              <>
                {!result && !running && (
                  <div className="output-placeholder">
                    <Activity size={20} />
                    Run an example to see execution traces
                  </div>
                )}
                {running && (
                  <div className="output-placeholder running">
                    <Loader2 size={20} className="spin" />
                    <div>Collecting traces...</div>
                  </div>
                )}
                {result && (
                  <TraceTree traces={result.traces || []} />
                )}
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
