import { type DragEvent, useCallback, useRef, useState } from 'react';

// Tauri's own file drop is off (`dragDropEnabled: false` in tauri.conf.json)
// so the HTML5 drags the app uses — emails onto folders, Tag Board cards —
// work. A file dragged in from Finder therefore arrives as an HTML5 drag, and
// one that nothing handles makes the webview open the file in place of the app.

/** Whether a drag carries files (from Finder), not text or an in-app item. */
function carriesFiles(dataTransfer: DataTransfer | null | undefined): boolean {
  return Array.from(dataTransfer?.types ?? []).includes('Files');
}

/**
 * Accept files dropped anywhere on a composer: `onFiles` gets them, and
 * `isOver` is true while a file drag is over it. Drags without files pass
 * through untouched.
 */
export function useFileDrop(onFiles: (files: FileList) => void) {
  const [isOver, setIsOver] = useState(false);
  // dragenter/dragleave fire for every child crossed; count them.
  const depth = useRef(0);

  const onDragEnter = useCallback((e: DragEvent) => {
    if (!carriesFiles(e.dataTransfer)) return;
    depth.current += 1;
    setIsOver(true);
  }, []);

  const onDragOver = useCallback((e: DragEvent) => {
    if (!carriesFiles(e.dataTransfer)) return;
    // Without this the drop is refused and the webview opens the file.
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  }, []);

  const onDragLeave = useCallback((e: DragEvent) => {
    if (!carriesFiles(e.dataTransfer)) return;
    depth.current = Math.max(0, depth.current - 1);
    if (depth.current === 0) setIsOver(false);
  }, []);

  const onDrop = useCallback(
    (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer)) return;
      depth.current = 0;
      setIsOver(false);
      // The editor took it already (an image it inlines).
      if (e.defaultPrevented) return;
      e.preventDefault();
      if (e.dataTransfer.files.length > 0) onFiles(e.dataTransfer.files);
    },
    [onFiles],
  );

  return { isOver, dropProps: { onDragEnter, onDragOver, onDragLeave, onDrop } };
}

/**
 * Keep a file dropped where nothing accepts it from opening in place of the
 * app. Returns the function that removes the guard.
 */
export function installFileDropGuard(target: Window): () => void {
  const guard = (e: Event) => {
    if (carriesFiles((e as globalThis.DragEvent).dataTransfer)) e.preventDefault();
  };
  target.addEventListener('dragover', guard);
  target.addEventListener('drop', guard);
  return () => {
    target.removeEventListener('dragover', guard);
    target.removeEventListener('drop', guard);
  };
}
