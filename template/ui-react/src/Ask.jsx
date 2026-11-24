// Ask a question over the indexed documents (feature `ai`). The answer streams in as
// `ask-token` events, and the passages it was given are listed under it as citations.
// (No double curly braces in this folder: the template engine would eat them.)
import { useState } from 'react';
import { api, errorText } from './ipc.js';
import { listen } from '@tauri-apps/api/event';
import Speak from './Speak.jsx';
import Dictate from './Dictate.jsx';
import { provenance } from './provenance.js';
import { t } from './i18n.js';

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
      const r = await api.ask({ question });
      setAnswer(r.reply.text);
      setResult(r);
    } catch (e) {
      setError(errorText(e));
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
      t('ask-tokens', '{ $input } in / { $output } out tokens', { input: reply.input_tokens, output: reply.output_tokens }),
      reply.cached_tokens ? t('ask-cached-tokens', '{ $count } cached', { count: reply.cached_tokens }) : '',
      reply.cost_usd != null ? `$${reply.cost_usd.toFixed(4)}` : '',
      reply.attempts?.length ? t('ask-failed-tries', 'after { $count } failed tries', { count: reply.attempts.length }) : '',
      result.cached ? t('ask-from-cache', 'from the answer cache') : '',
    ]
      .filter(Boolean)
      .join(' · ');
  return (
    <section>
      <h2>{t('ask-title', 'Ask')}</h2>
      <div className="row">
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && run()}
          placeholder={t('ask-placeholder', 'Ask about your documents')}
          aria-label={t('ask-question', 'Question')}
        />
        <button onClick={run} disabled={busy}>
          {busy ? t('ask-thinking', 'Thinking…') : t('ask-go', 'Ask')}
        </button>
        {features?.stt && <Dictate onText={setQuestion} />}
      </div>
      {error && (
        <p className="status error" role="alert">
          {error}
        </p>
      )}
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
                  {h.page ? t('search-page', ', page { $page }', { page: h.page }) : ''}
                </span>
              </li>
            ))}
          </ol>
        </>
      )}
    </section>
  );
}
