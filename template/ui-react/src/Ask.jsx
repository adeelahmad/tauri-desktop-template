// Ask a question over the indexed documents (feature `ai`). The answer streams in as
// `ask-token` events, and the passages it was given are listed under it as citations.
// (No double curly braces in this folder: the template engine would eat them.)
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import Speak from './Speak.jsx';
import Dictate from './Dictate.jsx';
import { provenance } from './provenance.js';

export default function Ask({ features }) {
  const [question, setQuestion] = useState('');
  const [answer, setAnswer] = useState('');
  const [result, setResult] = useState(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const run = async () => {
    if (!question.trim() || busy) return;
    setBusy(true);
    setError('');
    setAnswer('');
    setResult(null);
    const unlisten = await listen('ask-token', (e) => setAnswer((a) => a + e.payload));
    try {
      const r = await invoke('ask', { question });
      setAnswer(r.reply.text);
      setResult(r);
    } catch (e) {
      setError(String(e));
    } finally {
      unlisten();
      setBusy(false);
    }
  };
  const reply = result?.reply;
  const usage =
    reply &&
    [
      `${reply.provider || ''} ${reply.model}`.trim(),
      `${reply.input_tokens} in / ${reply.output_tokens} out tokens`,
      reply.cached_tokens ? `${reply.cached_tokens} cached` : '',
      reply.cost_usd != null ? `$${reply.cost_usd.toFixed(4)}` : '',
      reply.attempts?.length ? `after ${reply.attempts.length} failed ${reply.attempts.length === 1 ? 'try' : 'tries'}` : '',
      result.cached ? 'from the answer cache' : '',
    ]
      .filter(Boolean)
      .join(' · ');
  return (
    <section>
      <h2>Ask</h2>
      <div className="row">
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && run()}
          placeholder="Ask about your documents"
          aria-label="Question"
        />
        <button onClick={run} disabled={busy}>
          {busy ? 'Thinking…' : 'Ask'}
        </button>
        {features?.stt && <Dictate onText={setQuestion} />}
      </div>
      {error && <p className="status error">{error}</p>}
      {answer && <div className="answer">{answer}</div>}
      {result && (
        <>
          <p className="hint" title={reply.attempts?.map((a) => `${a.provider} ${a.model}: ${a.error}`).join('\n')}>
            {usage} {features?.tts && <Speak text={answer} />}
          </p>
          {reply.warnings?.map((w) => (
            <p key={w} className="status error">
              {w}
            </p>
          ))}
          <ol className="sources">
            {result.sources.map((h, i) => (
              <li key={i}>
                <span className="meta" title={provenance(h.prov)}>
                  [{i + 1}] {h.source}
                  {h.page ? `, page ${h.page}` : ''}
                </span>
              </li>
            ))}
          </ol>
        </>
      )}
    </section>
  );
}
