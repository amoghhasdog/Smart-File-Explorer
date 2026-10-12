import { useState } from "react";
import { ipc, type FileId } from "../../lib/ipc";

interface Props {
  id: FileId;
  alt: string;
  className?: string;
}

/** Thumbnail with graceful fallback when none exists yet. */
export function Thumb({ id, alt, className }: Props) {
  const [failed, setFailed] = useState(false);
  if (failed) return null;
  return (
    <img
      className={className}
      src={ipc.thumbUrl(id)}
      alt={alt}
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}
