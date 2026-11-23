// One line on where a passage came from (chunk provenance from the backend).
export function provenance(p) {
  if (!p) return '';
  const parts = [p.extractor];
  if (p.sha256) parts.push(`sha256 ${p.sha256.slice(0, 12)}`);
  if (p.model) parts.push(p.model);
  if (p.ingested) parts.push(`indexed ${p.ingested.slice(0, 16).replace('T', ' ')} UTC`);
  return parts.filter(Boolean).join(' · ');
}
