import { vi } from "vitest";

// jsdom has no layout, so the real virtualiser measures a 0-height viewport
// and renders nothing. Tests render every row instead.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: (opts: { count: number; estimateSize: (i: number) => number }) => {
    const size = opts.estimateSize(0);
    return {
      getTotalSize: () => opts.count * size,
      getVirtualItems: () =>
        Array.from({ length: opts.count }, (_, index) => ({
          index,
          key: index,
          start: index * size,
          size,
          end: (index + 1) * size,
          lane: 0,
        })),
      scrollToIndex: () => {},
      measureElement: () => {},
    };
  },
}));
