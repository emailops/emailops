import { type RefObject, useEffect, useRef } from 'react';

/**
 * Scroll an element back to the top whenever `token` changes.
 *
 * Exists for panels that stay mounted while hidden. Keeping them mounted is
 * deliberate — it preserves interactive state such as expanded groups — but it
 * also preserves scroll offset, so a panel re-opened later comes back parked
 * wherever the user last left it rather than at its first item. On a phone,
 * where the panel is a drawer covering the screen, that reads as the app having
 * opened somewhere arbitrary.
 *
 * The caller decides what "re-opened" means by choosing the token: a counter
 * incremented on open resets only on open, whereas a boolean also resets on
 * close. Tokens are compared by identity.
 */
export function useScrollReset<T extends HTMLElement>(token: unknown): RefObject<T | null> {
  const ref = useRef<T | null>(null);
  // Compared explicitly rather than left to the dependency array so mounting
  // is not itself a reset — a fresh element is already at the top, and under
  // StrictMode's double-invoked effects an implicit reset would fire twice.
  const lastToken = useRef(token);

  useEffect(() => {
    if (lastToken.current === token) return;
    lastToken.current = token;
    // `scrollTop` rather than `scrollTo`: this is a jump, not an animation, and
    // it must not depend on smooth-scroll behaviour being available.
    if (ref.current) ref.current.scrollTop = 0;
  }, [token]);

  return ref;
}
