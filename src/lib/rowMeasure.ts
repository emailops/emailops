// Row measurement for the windowed email list.
//
// @tanstack/virtual-core measures every mounted row through a ResizeObserver and
// caches the result. It has no guard for a zero-sized box, so the moment an
// ancestor goes `display: none` — which App.tsx does to the whole inbox column
// while an email is open — every live row reports height 0 and the cache is
// overwritten with zeros. `getTotalSize()` then collapses and the library starts
// adjusting its own scroll offset to compensate, against a container that has no
// scroll box to follow it.
//
// A row that measures 0 is telling us about the container, not about the row.
// Keep the last height we actually saw.

export interface RowMeasurement {
  /** Height the browser just reported for the row's border box. */
  measured: number;
  /** Height already in the virtualizer's cache for this row, if any. */
  cached: number | undefined;
  /** The list's estimate for a row that has never been measured. */
  estimate: number;
}

export function measuredRowHeight({ measured, cached, estimate }: RowMeasurement): number {
  if (measured > 0) return measured;
  // `cached` can itself be 0 if a hidden measurement got through before this
  // guard existed — treat that as "no measurement" rather than propagating it.
  if (cached !== undefined && cached > 0) return cached;
  return estimate;
}
