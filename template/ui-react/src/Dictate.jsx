// Speech to text with Whisper on this computer (feature `stt`): a microphone button that
// records until pressed again, and a file picker for audio files. Audio is decoded and
// resampled to 16 kHz mono by the web view, sent as 16-bit PCM, and the text goes to
// `onText`. (No double curly braces in this folder: the template engine would eat them.)
import { useRef, useState } from 'react';
import { api, errorText } from './ipc.js';

const RATE = 16000;

// Any audio the web view can decode (WAV, MP3, M4A, WebM, Ogg) to base64 16-bit PCM.
async function toPcm16(blob) {
  const ctx = new AudioContext();
  let decoded;
  try {
    decoded = await ctx.decodeAudioData(await blob.arrayBuffer());
  } finally {
    ctx.close();
  }
  const frames = Math.max(1, Math.ceil(decoded.duration * RATE));
  const off = new OfflineAudioContext(1, frames, RATE);
  const src = off.createBufferSource();
  src.buffer = decoded;
  src.connect(off.destination);
  src.start();
  const mono = (await off.startRendering()).getChannelData(0);
  const pcm = new Int16Array(mono.length);
  for (let i = 0; i < mono.length; i++) pcm[i] = Math.max(-1, Math.min(1, mono[i])) * 0x7fff;
  let bin = '';
  const bytes = new Uint8Array(pcm.buffer);
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(bin);
}

export default function Dictate({ onText }) {
  const [state, setState] = useState('idle'); // idle | recording | working
  const [error, setError] = useState('');
  const rec = useRef(null);
  const file = useRef(null);

  const transcribe = async (blob) => {
    setState('working');
    try {
      const r = await api.transcribe({ audio: await toPcm16(blob) });
      if (r.text) onText(r.text);
      else setError('No speech heard.');
    } catch (e) {
      setError(errorText(e));
    } finally {
      setState('idle');
    }
  };

  const toggle = async () => {
    setError('');
    if (state === 'recording') return rec.current?.stop();
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const chunks = [];
      const r = new MediaRecorder(stream);
      r.ondataavailable = (e) => e.data.size && chunks.push(e.data);
      r.onstop = () => {
        stream.getTracks().forEach((t) => t.stop());
        transcribe(new Blob(chunks, { type: r.mimeType }));
      };
      rec.current = r;
      r.start();
      setState('recording');
    } catch (e) {
      setError(`Microphone: ${e.message || e}`);
    }
  };

  const label = { idle: '🎤', recording: '■ Stop', working: '…' }[state];
  return (
    <>
      <button
        onClick={toggle}
        disabled={state === 'working'}
        title={state === 'recording' ? 'Stop and transcribe' : 'Dictate'}
        aria-label="Dictate"
      >
        {label}
      </button>
      <button className="link" onClick={() => file.current?.click()} disabled={state !== 'idle'} title="Transcribe an audio file">
        Audio file
      </button>
      <input
        ref={file}
        type="file"
        accept="audio/*"
        hidden
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = '';
          if (f) {
            setError('');
            transcribe(f);
          }
        }}
      />
      {error && (
        <span className="status error" role="alert">
          {error}
        </span>
      )}
    </>
  );
}
