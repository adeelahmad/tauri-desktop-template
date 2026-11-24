// The {{Title}} page. Talk to the backend with invoke('<command>', { ... });
// `kitchen generate command <name>` makes a new command.
import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

export default function {{Name}}() {
  const [text, setText] = useState('');
  const load = async () => {
    try {
      const info = await invoke('app_info');
      setText(`${info.name} ${info.version}`);
    } catch (e) {
      setText(String(e));
    }
  };
  return (
    <section>
      <h2>{{Title}}</h2>
      <button onClick={load}>Load</button>
      <p className="hint">{text || 'Edit ui-react/src/{{Name}}.jsx.'}</p>
    </section>
  );
}
