import type { Chip } from "../../lib/ipc";

interface Props {
  chips: Chip[];
}

/** Shows how the backend understood the query, one chip per fragment. */
export function QueryChips({ chips }: Props) {
  if (chips.length === 0) return null;
  return (
    <ul className="chips" aria-label="Query interpretation">
      {chips.map((c, i) => (
        <li key={`${c.kind}-${i}`} className={`chip chip-${c.kind}`}>
          <span className="chip-kind">{c.kind}</span>
          {c.label}
        </li>
      ))}
    </ul>
  );
}
