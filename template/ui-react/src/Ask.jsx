// Ask a question over the indexed documents (feature `ai`). The answer streams in as
// `ask-token` events, and the passages it was given are listed under it as citations.
// (No double curly braces in this folder: the template engine would eat them.)
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import Speak from './Speak.jsx';
import Dictate from './Dictate.jsx';

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
  const usage = result && `${result.reply.model} · ${result.reply.input_tokens} in / ${result.reply.output_tokens} out tokens`;
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
          <p className="hint">
            {usage} {features?.tts && <Speak text={answer} />}
          </p>
          <ol className="sources">
            {result.sources.map((h, i) => (
              <li key={i}>
                <span className="meta">
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
