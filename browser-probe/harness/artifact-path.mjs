import { resolve } from 'node:path';

// The build runner supplies only the programs pinned for this runtime batch.
export function artifactPath(value) {
  const paths = JSON.parse(process.env.MIOPUNCH_TEST_ARTIFACTS || '{}');
  const requested = resolve(value);
  if (paths[requested]) return paths[requested];
  const offset = requested.indexOf('/target/');
  if (offset >= 0) {
    for (const [original, pin] of Object.entries(paths)) {
      const sourceOffset = original.indexOf('/target/');
      if (sourceOffset >= 0 && original.slice(sourceOffset) === requested.slice(offset)) {
        return requested.slice(0, offset) + pin.slice(sourceOffset);
      }
    }
  }
  return value;
}
