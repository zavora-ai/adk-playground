import { useState, useEffect, useCallback, useRef, useMemo, lazy, Suspense } from 'react';
import { type OnMount } from '@monaco-editor/react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import rehypeHighlight from 'rehype-highlight';
import { Play, Square, Loader2, BookOpen, ChevronRight, ChevronDown, Terminal, Clock, CheckCircle, XCircle, Keyboard, Lock, Globe, Activity, Cpu, Wrench, MessageSquare, AlertTriangle, Bot, Github, Star, Zap, DollarSign, Hash, Sun, Moon, Monitor, Volume2, Heart, X } from 'lucide-react';
import './App.css';

const Editor = lazy(() => import('@monaco-editor/react'));
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
  input_tokens?: number;
  output_tokens?: number;
  thinking_tokens?: number;
  cache_read_tokens?: number;
  model_name?: string;
  thinking_text?: string;
  cost?: number;
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
  usage: Hash,
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
        {event.model_name && (
          <>
            <span className="trace-detail-key">Model</span>
            <span className="trace-detail-val">{event.model_name}</span>
          </>
        )}
        {event.input_tokens != null && (
          <>
            <span className="trace-detail-key">Input Tokens</span>
            <span className="trace-detail-val">{event.input_tokens.toLocaleString()}</span>
          </>
        )}
        {event.output_tokens != null && (
          <>
            <span className="trace-detail-key">Output Tokens</span>
            <span className="trace-detail-val">{event.output_tokens.toLocaleString()}</span>
          </>
        )}
        {event.thinking_tokens != null && event.thinking_tokens > 0 && (
          <>
            <span className="trace-detail-key">Thinking Tokens</span>
            <span className="trace-detail-val">{event.thinking_tokens.toLocaleString()}</span>
          </>
        )}
        {event.cache_read_tokens != null && event.cache_read_tokens > 0 && (
          <>
            <span className="trace-detail-key">Cache Read</span>
            <span className="trace-detail-val">{event.cache_read_tokens.toLocaleString()}</span>
          </>
        )}
        {event.cost != null && (
          <>
            <span className="trace-detail-key">Cost</span>
            <span className="trace-detail-val trace-cost-val">${event.cost < 0.01 ? '<0.01' : event.cost.toFixed(4)}</span>
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
      {event.thinking_text && (
        <div className="trace-detail-section">
          <div className="trace-detail-section-label">💭 Thinking</div>
          <pre className="trace-detail-pre trace-thinking-text">{event.thinking_text}</pre>
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
                        {child.event.input_tokens != null && (
                          <span className="trace-tokens-badge">
                            {child.event.input_tokens.toLocaleString()} in / {(child.event.output_tokens ?? 0).toLocaleString()} out
                          </span>
                        )}
                        {child.event.cost != null && (
                          <span className="trace-cost-badge">
                            ${child.event.cost < 0.01 ? '<0.01' : child.event.cost.toFixed(4)}
                          </span>
                        )}
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

/** Parse a tracing-subscriber JSON object into a TraceEvent for the frontend */
function parseTraceJson(json: Record<string, unknown>, index: number): TraceEvent | null {
  const level = (json.level as string || 'info').toLowerCase();
  const target = (json.target as string) || '';
  const fields = (json.fields as Record<string, unknown>) || {};
  const message = (fields.message as string) || '';

  // Skip noisy internal traces
  if (['hyper', 'reqwest', 'h2', 'rustls', 'tonic', 'tower', 'mio', 'want'].some(p => target.startsWith(p))) {
    return null;
  }

  const spans = json.spans as Array<Record<string, unknown>> | undefined;
  const span = json.span as Record<string, unknown> | undefined;
  const spanName = (span?.name as string) ||
    (spans?.length ? (spans[spans.length - 1].name as string) : '') || '';

  const agent = spans?.find(s => s['agent.name'] || s['gcp.vertex.agent.agent_name'])
    ? String(spans.find(s => s['agent.name'])?.['agent.name'] || spans.find(s => s['gcp.vertex.agent.agent_name'])?.['gcp.vertex.agent.agent_name'])
    : (fields['agent.name'] as string) || undefined;

  const tool = (fields['tool.name'] as string) || undefined;
  const detail = ((fields['tool.args'] || fields['tool.result']) as string) || undefined;

  // --- Token usage extraction ---
  let input_tokens: number | undefined;
  let output_tokens: number | undefined;
  let thinking_tokens: number | undefined;
  let cache_read_tokens: number | undefined;
  let model_name: string | undefined;
  let thinking_text: string | undefined;
  let cost: number | undefined;

  // Extract model name from spans
  const findSpanField = (field: string): unknown => {
    if (span && span[field] != null) return span[field];
    if (spans) {
      for (let i = spans.length - 1; i >= 0; i--) {
        if (spans[i][field] != null) return spans[i][field];
      }
    }
    return undefined;
  };

  model_name = (findSpanField('model.name') as string) ||
    (findSpanField('gen_ai.request.model') as string) || undefined;

  // Handle span CLOSE events (message === "close") — these have recorded fields
  if (message === 'close') {
    const usageInput = span?.['gen_ai.usage.input_tokens'] as number | undefined;
    const usageOutput = span?.['gen_ai.usage.output_tokens'] as number | undefined;

    if (usageInput != null || usageOutput != null) {
      input_tokens = usageInput;
      output_tokens = usageOutput;
      thinking_tokens = span?.['gen_ai.usage.thinking_tokens'] as number | undefined;
      cache_read_tokens = span?.['gen_ai.usage.cache_read_tokens'] as number | undefined;
      cost = estimateCostFromTokens(model_name, input_tokens, output_tokens, thinking_tokens);

      return {
        timestamp_ms: index,
        level,
        name: 'LLM Usage',
        message: `Tokens: ${input_tokens ?? 0} in / ${output_tokens ?? 0} out${thinking_tokens ? ` / ${thinking_tokens} thinking` : ''}`,
        kind: 'usage',
        target,
        input_tokens,
        output_tokens,
        thinking_tokens,
        cache_read_tokens,
        model_name,
        cost,
      };
    }
    // Skip other span close events
    return null;
  }

  // Extract token data from gcp.vertex.agent.llm_response (recorded on spans)
  const llmResponseStr = (findSpanField('gcp.vertex.agent.llm_response') as string) || undefined;
  if (llmResponseStr) {
    try {
      const resp = JSON.parse(llmResponseStr);
      const usage = resp.usageMetadata;
      if (usage) {
        input_tokens = usage.promptTokenCount;
        output_tokens = usage.candidatesTokenCount;
        thinking_tokens = usage.thinkingTokenCount || usage.thoughtsTokenCount;
        cache_read_tokens = usage.cachedContentTokenCount;
      }
      // Extract thinking text
      const candidates = resp.candidates as Array<{ content?: { parts?: Array<{ thought?: boolean; text?: string }> } }>;
      if (candidates) {
        for (const c of candidates) {
          for (const part of c.content?.parts || []) {
            if (part.thought && part.text) {
              thinking_text = part.text;
            }
          }
        }
      }
    } catch { /* ignore parse errors */ }
  }

  cost = estimateCostFromTokens(model_name, input_tokens, output_tokens, thinking_tokens);

  const msgLower = message.toLowerCase();
  let kind = 'info';
  if (spanName.includes('agent.execute') || msgLower.includes('agent execution')) kind = 'agent';
  else if (spanName.includes('llm') || msgLower.includes('llm_response') || msgLower.includes('llm_call')) kind = 'llm';
  else if (msgLower.includes('tool_call') || (tool && msgLower.includes('tool_call'))) kind = 'tool_call';
  else if (msgLower.includes('tool_result')) kind = 'tool_result';
  else if (tool) kind = 'tool_call';
  else if (msgLower.includes('warn')) kind = 'warn';

  return {
    timestamp_ms: index,
    level,
    name: spanName || target,
    message,
    agent,
    tool,
    detail,
    kind,
    target,
    input_tokens,
    output_tokens,
    thinking_tokens,
    cache_read_tokens,
    model_name,
    thinking_text,
    cost,
  };
}

/** Estimate cost from token counts and model name */
function estimateCostFromTokens(model?: string, input?: number, output?: number, thinking?: number): number | undefined {
  if (!input && !output) return undefined;
  const inp = input ?? 0;
  const out = output ?? 0;
  const think = thinking ?? 0;
  // Prices per 1M tokens: (input, output)
  let ip = 0.25, op = 1.50; // default: flash-lite pricing
  if (model) {
    if (model.includes('flash-lite')) { ip = 0.25; op = 1.50; }
    else if (model.includes('flash')) { ip = 0.15; op = 0.60; }
    else if (model.includes('pro')) { ip = 1.25; op = 5.00; }
    else if (model.includes('gpt-4.1-mini')) { ip = 0.40; op = 1.60; }
    else if (model.includes('gpt-4.1')) { ip = 2.00; op = 8.00; }
    else if (model.includes('gpt-4o-mini')) { ip = 0.15; op = 0.60; }
    else if (model.includes('gpt-4o')) { ip = 2.50; op = 10.00; }
    else if (model.includes('gpt-5-mini')) { ip = 0.40; op = 1.60; }
    else if (model.includes('o4-mini')) { ip = 1.10; op = 4.40; }
    else if (model.includes('o3-mini')) { ip = 1.10; op = 4.40; }
    else if (model.includes('claude-sonnet-4-5')) { ip = 2.00; op = 10.00; }
    else if (model.includes('claude-sonnet-4') || model.includes('claude-3-7-sonnet')) { ip = 3.00; op = 15.00; }
    else if (model.includes('claude-3-5-haiku')) { ip = 0.80; op = 4.00; }
    else if (model.includes('claude-3-5-sonnet')) { ip = 3.00; op = 15.00; }
    else if (model.includes('deepseek-chat')) { ip = 0.27; op = 1.10; }
    else if (model.includes('deepseek-reasoner')) { ip = 0.55; op = 2.19; }
    else if (model.includes('mistral-medium')) { ip = 0.40; op = 1.20; }
    else if (model.includes('mistral-small')) { ip = 0.10; op = 0.30; }
    else if (model.includes('mistral-large')) { ip = 2.00; op = 6.00; }
    else if (model.includes('grok-3-mini')) { ip = 0.30; op = 0.50; }
    else if (model.includes('grok-3')) { ip = 3.00; op = 15.00; }
  }
  // Thinking tokens billed as output
  return (inp * ip + (out + think) * op) / 1_000_000;
}

function extractTokenUsage(stdout: string): { input_tokens?: number; output_tokens?: number; total_tokens?: number } {
  let input_tokens: number | undefined;
  let output_tokens: number | undefined;
  let total_tokens: number | undefined;

  for (const line of stdout.split('\n')) {
    const lower = line.toLowerCase();
    const nums = line.match(/\d+/g)?.map(Number);
    if (!nums?.length) continue;
    const last = nums[nums.length - 1];

    if (lower.includes('prompt') && lower.includes('token')) input_tokens = last;
    else if ((lower.includes('candidate') || lower.includes('output') || lower.includes('completion')) && lower.includes('token')) output_tokens = last;
    else if (lower.includes('total') && lower.includes('token')) total_tokens = last;
    else if (lower.includes('input:') && lower.includes('output:')) {
      for (const part of line.split(',')) {
        const pl = part.toLowerCase();
        const pn = part.match(/\d+/g)?.map(Number);
        if (pn?.length) {
          if (pl.includes('input')) input_tokens = pn[pn.length - 1];
          if (pl.includes('output')) output_tokens = pn[pn.length - 1];
        }
      }
    }
  }
  if (!total_tokens && input_tokens && output_tokens) total_tokens = input_tokens + output_tokens;
  return { input_tokens, output_tokens, total_tokens };
}

function estimateCost(model: string, input?: number, output?: number): number | undefined {
  if (!input || !output) return undefined;
  let ip: number, op: number;
  if (model.includes('flash-lite')) { ip = 0.25; op = 1.50; }
  else if (model.includes('flash')) { ip = 0.15; op = 0.60; }
  else if (model.includes('pro')) { ip = 1.25; op = 5.00; }
  else if (model.includes('gpt-4o-mini')) { ip = 0.15; op = 0.60; }
  else if (model.includes('gpt-4o')) { ip = 2.50; op = 10.00; }
  else if (model.includes('claude-3-5-haiku')) { ip = 0.80; op = 4.00; }
  else if (model.includes('claude-3-5-sonnet') || model.includes('claude-3-7-sonnet')) { ip = 3.00; op = 15.00; }
  else return undefined;
  return (input * ip + output * op) / 1_000_000;
}

type Theme = 'dark' | 'light' | 'system';
const THEME_KEY = 'adk-playground-theme';

function getStoredTheme(): Theme {
  const stored = localStorage.getItem(THEME_KEY);
  if (stored === 'dark' || stored === 'light' || stored === 'system') return stored;
  return 'system';
}

function getResolvedTheme(theme: Theme): 'dark' | 'light' {
  if (theme !== 'system') return theme;
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

function applyTheme(theme: Theme) {
  const resolved = getResolvedTheme(theme);
  document.documentElement.setAttribute('data-theme', resolved);
}

const THEME_CYCLE: Theme[] = ['dark', 'light', 'system'];
const THEME_ICONS: Record<Theme, typeof Moon> = { dark: Moon, light: Sun, system: Monitor };
const THEME_LABELS: Record<Theme, string> = { dark: 'Dark', light: 'Light', system: 'System' };

/** Extract audio markers from stdout: <!--AUDIO_URL:/api/audio/file.wav--> */
const AUDIO_URL_RE = /<!--AUDIO_URL:([\S]+?)-->/g;
/** Streaming audio markers */
const AUDIO_CHUNK_RE = /<!--AUDIO_CHUNK:([\S]+?)-->/g;
const AUDIO_STREAM_START_RE = /<!--AUDIO_STREAM_START:(\d+)-->/;
const AUDIO_STREAM_END_RE = /<!--AUDIO_STREAM_END-->/;

/** Streaming audio player using Web Audio API — plays PCM16 chunks as they arrive */
function StreamingAudioPlayer({ chunks, sampleRate, isStreaming }: {
  chunks: string[];
  sampleRate: number;
  isStreaming: boolean;
}) {
  const ctxRef = useRef<AudioContext | null>(null);
  const nextTimeRef = useRef(0);
  const processedRef = useRef(0);
  const [playing, setPlaying] = useState(false);
  const [audioPlaying, setAudioPlaying] = useState(false); // true while Web Audio is still outputting
  const [elapsed, setElapsed] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval>>(undefined);

  // Process new chunks as they arrive
  useEffect(() => {
    if (chunks.length <= processedRef.current) return;

    if (!ctxRef.current) {
      ctxRef.current = new AudioContext({ sampleRate });
      nextTimeRef.current = ctxRef.current.currentTime + 0.05; // small initial buffer
      setPlaying(true);
      setAudioPlaying(true);
      timerRef.current = setInterval(() => {
        if (ctxRef.current) {
          setElapsed(ctxRef.current.currentTime);
          // Check if all scheduled audio has finished playing
          if (nextTimeRef.current > 0 && ctxRef.current.currentTime >= nextTimeRef.current) {
            setAudioPlaying(false);
          } else {
            setAudioPlaying(true);
          }
        }
      }, 200);
    }

    const ctx = ctxRef.current;
    const newChunks = chunks.slice(processedRef.current);
    processedRef.current = chunks.length;

    for (const b64 of newChunks) {
      try {
        const raw = atob(b64);
        const bytes = new Uint8Array(raw.length);
        for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);

        // PCM16 LE → Float32
        const samples = bytes.length / 2;
        const buffer = ctx.createBuffer(1, samples, sampleRate);
        const channel = buffer.getChannelData(0);
        for (let i = 0; i < samples; i++) {
          const lo = bytes[i * 2];
          const hi = bytes[i * 2 + 1];
          const val = (hi << 8) | lo;
          channel[i] = (val >= 0x8000 ? val - 0x10000 : val) / 32768;
        }

        const source = ctx.createBufferSource();
        source.buffer = buffer;
        source.connect(ctx.destination);

        const startAt = Math.max(nextTimeRef.current, ctx.currentTime);
        source.start(startAt);
        nextTimeRef.current = startAt + buffer.duration;
        setAudioPlaying(true);
      } catch { /* skip bad chunks */ }
    }
  }, [chunks, sampleRate]);

  // Cleanup on unmount or when streaming ends
  useEffect(() => {
    if (!isStreaming && !chunks.length) return;
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [isStreaming, chunks.length]);

  // The player is "live" while either new chunks are arriving OR buffered audio is still playing
  const isLive = isStreaming || audioPlaying;

  if (!playing && !isStreaming) return null;

  return (
    <div className="audio-stream-block">
      <div className="audio-stream-header">
        <Volume2 size={14} />
        <span>Live Audio</span>
        {isLive && <span className="audio-stream-live">● LIVE</span>}
        {!isLive && <span className="audio-stream-done">Complete</span>}
      </div>
      <div className="audio-stream-bar">
        <div className="audio-stream-wave">
          {isLive && Array.from({ length: 5 }).map((_, i) => (
            <span key={i} className="audio-wave-bar" style={{ animationDelay: `${i * 0.1}s` }} />
          ))}
        </div>
        <span className="audio-stream-time">{elapsed.toFixed(1)}s</span>
      </div>
    </div>
  );
}

/** Typewriter hook — reveals text progressively to simulate streaming */
function useTypewriter(text: string, active: boolean, charsPerTick = 12, intervalMs = 16): string {
  const [displayed, setDisplayed] = useState('');
  const indexRef = useRef(0);

  useEffect(() => {
    if (!active) { setDisplayed(text); indexRef.current = text.length; return; }
    // Reset when text identity changes (new thinking block)
    indexRef.current = 0;
    setDisplayed('');
  }, [text, active]);

  useEffect(() => {
    if (!active || indexRef.current >= text.length) return;
    const id = setInterval(() => {
      indexRef.current = Math.min(indexRef.current + charsPerTick, text.length);
      setDisplayed(text.slice(0, indexRef.current));
      if (indexRef.current >= text.length) clearInterval(id);
    }, intervalMs);
    return () => clearInterval(id);
  }, [text, active, charsPerTick, intervalMs]);

  return displayed;
}

function ThinkingBlock({ content, animate }: { content: string; animate: boolean }) {
  const displayed = useTypewriter(content, animate, 18, 12);
  return (
    <div className="thinking-block">
      <div className="thinking-label">💭 Thinking</div>
      <div className="thinking-content">
        {displayed}
        {animate && displayed.length < content.length && <span className="thinking-cursor">▊</span>}
      </div>
    </div>
  );
}

function OutputContent({ stdout, isStreaming }: { stdout: string; isError?: boolean; isStreaming?: boolean }) {
  // Parse streaming audio state from stdout
  const audioState = useMemo(() => {
    const streamStartMatch = stdout.match(AUDIO_STREAM_START_RE);
    const hasStreamEnd = AUDIO_STREAM_END_RE.test(stdout);
    const sampleRate = streamStartMatch ? parseInt(streamStartMatch[1], 10) : 0;
    const isActive = !!streamStartMatch && !hasStreamEnd;

    const chunks: string[] = [];
    let m: RegExpExecArray | null;
    const chunkRe = new RegExp(AUDIO_CHUNK_RE.source, 'g');
    while ((m = chunkRe.exec(stdout)) !== null) {
      chunks.push(m[1]);
    }

    return { sampleRate, isActive, chunks, hasStream: !!streamStartMatch };
  }, [stdout]);

  // Extract thinking blocks and text parts for styled rendering
  const contentParts = useMemo(() => {
    const thinkingRe = /<!--THINKING_START-->\n?([\s\S]*?)<!--THINKING_END-->\n?/g;
    // Strip audio markers first
    const cleaned = stdout
      .replace(/<!--AUDIO_STREAM_START:\d+-->\n?/g, '')
      .replace(/<!--AUDIO_CHUNK:[\S]+?-->\n?/g, '')
      .replace(/<!--AUDIO_STREAM_END-->\n?/g, '');

    const parts: Array<{ type: 'text' | 'thinking'; content: string }> = [];
    let lastIdx = 0;
    let m: RegExpExecArray | null;
    while ((m = thinkingRe.exec(cleaned)) !== null) {
      if (m.index > lastIdx) {
        parts.push({ type: 'text', content: cleaned.slice(lastIdx, m.index) });
      }
      parts.push({ type: 'thinking', content: m[1].trim() });
      lastIdx = thinkingRe.lastIndex;
    }
    if (lastIdx < cleaned.length) {
      parts.push({ type: 'text', content: cleaned.slice(lastIdx) });
    }
    // Merge consecutive thinking blocks into one
    const merged: typeof parts = [];
    for (const p of parts) {
      const prev = merged[merged.length - 1];
      if (p.type === 'thinking' && prev?.type === 'thinking') {
        prev.content += ' ' + p.content;
      } else {
        merged.push({ ...p });
      }
    }
    return merged;
  }, [stdout]);


  return (
    <>
      {audioState.hasStream && (audioState.isActive || audioState.chunks.length > 0) && (
        <StreamingAudioPlayer
          chunks={audioState.chunks}
          sampleRate={audioState.sampleRate}
          isStreaming={audioState.isActive && !!isStreaming}
        />
      )}
      {contentParts.map((cp, ci) => {
        if (cp.type === 'thinking') {
          return (
            <div key={`t-${ci}`} className="thinking-block">
              <div className="thinking-label">💭 Thinking</div>
              <div className="thinking-content">{cp.content}</div>
            </div>
          );
        }
        // For text parts, handle audio URLs within them
        const textContent = cp.content;
        const innerParts: Array<{ type: 'text'; content: string } | { type: 'audio'; url: string }> = [];
        let idx = 0;
        let m2: RegExpExecArray | null;
        const audioRe = new RegExp(AUDIO_URL_RE.source, 'g');
        while ((m2 = audioRe.exec(textContent)) !== null) {
          if (m2.index > idx) {
            innerParts.push({ type: 'text', content: textContent.slice(idx, m2.index) });
          }
          const url = m2[1].startsWith('http') ? m2[1] : `${API}${m2[1]}`;
          innerParts.push({ type: 'audio', url });
          idx = audioRe.lastIndex;
        }
        if (idx < textContent.length) {
          innerParts.push({ type: 'text', content: textContent.slice(idx) });
        }
        return innerParts.map((part, pi) => {
          if (part.type === 'audio') {
            if (audioState.isActive && isStreaming) return null;
            return (
              <div key={`${ci}-a-${pi}`} className="audio-block">
                <div className="audio-label"><Volume2 size={14} /> Audio Output</div>
                <audio controls src={part.url} />
              </div>
            );
          }
          const text = part.content.trim();
          if (!text) return null;
          return (
            <Markdown key={`${ci}-${pi}`} remarkPlugins={[remarkGfm, remarkMath]} rehypePlugins={[rehypeKatex, rehypeHighlight]}>
              {text}
            </Markdown>
          );
        });
      })}
    </>
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
  const [statusText, setStatusText] = useState('');
  const [theme, setTheme] = useState<Theme>(getStoredTheme);
  const [sponsorDismissed, setSponsorDismissed] = useState(() => sessionStorage.getItem('sponsor-dismissed') === '1');
  const runRef = useRef<() => void>(undefined);
  const abortRef = useRef<AbortController | null>(null);

  const isPublic = serverMode?.mode === 'public';

  // Apply theme on change and listen for system preference changes
  useEffect(() => {
    applyTheme(theme);
    localStorage.setItem(THEME_KEY, theme);
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => { if (theme === 'system') applyTheme('system'); };
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [theme]);

  const resolvedTheme = getResolvedTheme(theme);
  const cycleTheme = () => {
    const idx = THEME_CYCLE.indexOf(theme);
    setTheme(THEME_CYCLE[(idx + 1) % THEME_CYCLE.length]);
  };

  const stopRun = useCallback(() => {
    if (abortRef.current) {
      abortRef.current.abort();
      abortRef.current = null;
    }
  }, []);

  const runCode = useCallback(async () => {
    if (running || !code.trim()) return;
    if (isPublic && !selectedId) return;
    setRunning(true);
    setResult(null);
    setStatusText('Connecting...');
    setActiveTab('output');

    const abort = new AbortController();
    abortRef.current = abort;

    // Accumulate streaming data
    let stdout = '';
    let stderr = '';
    const traces: TraceEvent[] = [];
    let summary: RunSummary | undefined;
    let success = true;

    try {
      const res = await fetch(`${API}/api/run-stream`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code, example_id: selectedId || 'custom' }),
        signal: abort.signal,
      });

      const reader = res.body?.getReader();
      if (!reader) throw new Error('No response body');

      const decoder = new TextDecoder();
      let buffer = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';

        let eventType = '';
        for (const line of lines) {
          if (line.startsWith('event: ')) {
            eventType = line.slice(7).trim();
          } else if (line.startsWith('data: ')) {
            const data = line.slice(6);
            switch (eventType) {
              case 'status':
                setStatusText(data);
                break;
              case 'stdout':
                stdout += data + '\n';
                setResult(prev => ({
                  success: true,
                  stdout,
                  stderr: prev?.stderr || '',
                  duration_ms: 0,
                  traces: prev?.traces || [],
                  summary: prev?.summary,
                }));
                break;
              case 'stderr':
                stderr += data + '\n';
                setResult(prev => ({
                  success: true,
                  stdout: prev?.stdout || '',
                  stderr,
                  duration_ms: 0,
                  traces: prev?.traces || [],
                  summary: prev?.summary,
                }));
                break;
              case 'trace':
                try {
                  const json = JSON.parse(data);
                  const evt = parseTraceJson(json, traces.length);
                  if (evt) {
                    traces.push(evt);
                    // Inject thinking text into stdout so it streams in the Output tab
                    if (evt.thinking_text) {
                      stdout += `<!--THINKING_START-->\n${evt.thinking_text}\n<!--THINKING_END-->\n`;
                    }
                    setResult(prev => ({
                      success: true,
                      stdout: stdout || prev?.stdout || '',
                      stderr: prev?.stderr || '',
                      duration_ms: 0,
                      traces: [...traces],
                      summary: prev?.summary,
                    }));
                  }
                } catch { /* skip unparseable trace lines */ }
                break;
              case 'error':
                success = false;
                stderr += data + '\n';
                setResult({
                  success: false,
                  stdout,
                  stderr,
                  duration_ms: 0,
                  traces: [...traces],
                });
                break;
              case 'done':
                try {
                  const doneData = JSON.parse(data);
                  success = doneData.success ?? true;
                  summary = {
                    compile_ms: doneData.compile_ms || 0,
                    run_ms: doneData.run_ms || 0,
                    model: doneData.model,
                  };
                } catch { /* ignore */ }
                break;
            }
            eventType = '';
          }
        }
      }
    } catch (e) {
      if ((e as Error).name !== 'AbortError') {
        stderr += `Connection error: ${e}\n`;
        success = false;
      }
    }

    // Extract token usage from stdout AND traces
    const { input_tokens, output_tokens, total_tokens } = extractTokenUsage(stdout);
    // Also aggregate token data from trace events (span close events with gen_ai.usage.*)
    let traceInputTokens = 0;
    let traceOutputTokens = 0;
    let traceThinkingTokens = 0;
    let traceCost = 0;
    let traceModel: string | undefined;
    for (const t of traces) {
      if (t.input_tokens != null) traceInputTokens += t.input_tokens;
      if (t.output_tokens != null) traceOutputTokens += t.output_tokens;
      if (t.thinking_tokens != null) traceThinkingTokens += t.thinking_tokens;
      if (t.cost != null) traceCost += t.cost;
      if (t.model_name && !traceModel) traceModel = t.model_name;
    }

    if (summary) {
      // Prefer trace-derived tokens over stdout-parsed ones
      summary.input_tokens = input_tokens || (traceInputTokens > 0 ? traceInputTokens : undefined);
      summary.output_tokens = output_tokens || (traceOutputTokens > 0 ? traceOutputTokens : undefined);
      summary.total_tokens = total_tokens || (traceInputTokens + traceOutputTokens > 0 ? traceInputTokens + traceOutputTokens : undefined);
      if (!summary.model && traceModel) summary.model = traceModel;
      if (summary.model) {
        summary.cost_estimate = traceCost > 0 ? traceCost : estimateCost(summary.model, summary.input_tokens, summary.output_tokens);
      }
    }

    setResult({
      success,
      stdout: stdout.trimEnd(),
      stderr: stderr.trimEnd(),
      duration_ms: summary ? summary.compile_ms + summary.run_ms : 0,
      traces: [...traces],
      summary,
    });
    setStatusText('');
    setRunning(false);
    abortRef.current = null;
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
          <h1>⚡ ADK-Rust Playground</h1>
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
          <button className="theme-toggle" onClick={cycleTheme} title={`Theme: ${THEME_LABELS[theme]}`}>
            {(() => { const Icon = THEME_ICONS[theme]; return <Icon size={14} />; })()}
            <span className="theme-label">{THEME_LABELS[theme]}</span>
          </button>
          <a href="https://github.com/zavora-ai/adk-rust" target="_blank" rel="noopener noreferrer" className="github-star-link">
            <Github size={15} />
            <Star size={12} className="star-icon" />
            Star on GitHub
          </a>
          <span className="shortcut-hint">
            <Keyboard size={12} />
            {MOD_KEY}+Enter
          </span>
          {running ? (
            <>
              <button className="run-btn running" disabled>
                <Loader2 size={16} className="spin" />
                {statusText.includes('running') ? 'Running...' : 'Compiling...'}
              </button>
              <button className="stop-btn" onClick={stopRun} title="Stop execution">
                <Square size={14} />
              </button>
            </>
          ) : (
            <button
              className="run-btn"
              onClick={runCode}
              disabled={!code.trim() || backendStatus === 'offline'}
            >
              <Play size={16} />
              Run
            </button>
          )}
        </div>
      </header>

      {!sponsorDismissed && (
        <div className="sponsor-banner">
          <Heart size={13} className="sponsor-heart" />
          <span className="sponsor-text">
            Help keep ADK-Rust free and growing.
            <a
              href="https://github.com/sponsors/zavora-ai"
              target="_blank"
              rel="noopener noreferrer"
            >
              Become a Gold Sponsor
            </a>
            — $100/mo gets your logo on the ADK-Rust website & GitHub page, plus a copy of the Official ADK-Rust Blueprint book.
          </span>
          <button
            className="sponsor-dismiss"
            onClick={() => { setSponsorDismissed(true); sessionStorage.setItem('sponsor-dismissed', '1'); }}
            title="Dismiss"
          >
            <X size={13} />
          </button>
        </div>
      )}

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
            <Suspense fallback={<div style={{ padding: 24, color: 'var(--text-dim)' }}>Loading editor...</div>}>
            <Editor
              height="100%"
              defaultLanguage="rust"
              theme={resolvedTheme === 'dark' ? 'vs-dark' : 'light'}
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
            </Suspense>
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
                {running && !result && (
                  <div className="output-placeholder running">
                    <Loader2 size={20} className="spin" />
                    <div>
                      <div>{statusText || 'Connecting...'}</div>
                      <div className="output-sub">First builds take longer</div>
                    </div>
                  </div>
                )}
                {running && result && (
                  <div className="streaming-output">
                    <div className="streaming-status">
                      <Loader2 size={14} className="spin" />
                      <span>{statusText || 'Streaming...'}</span>
                    </div>
                    <div className="output-markdown">
                      {result.stdout && <OutputContent stdout={result.stdout} isStreaming={true} />}
                      {result.stderr && <pre className="stderr-block">{result.stderr}</pre>}
                    </div>
                  </div>
                )}
                {!running && result && (
                  <div className={`output-markdown ${result.success ? '' : 'error'}`}>
                    {result.stdout && <OutputContent stdout={result.stdout} isError={!result.success} />}
                    {result.stderr && <pre className="stderr-block">{result.stderr}</pre>}
                    {!result.stdout && !result.stderr && (
                      <p className="no-output">(no output)</p>
                    )}
                  </div>
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
                {running && !result && (
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
