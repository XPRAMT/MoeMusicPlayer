/** Mark a text for the CSS marquee (`.dock-marquee` / `.dock-marquee-text` in app.css) only when it overflows its slot:
 * the play dock title/artist and the device names in the output channel-map matrix.
 * The scroll is plain linear and moves sub-pixel (user decision 2026-10-24); the pauses at both ends hold one
 * transform, so they change nothing and present nothing. */
export function dockMarquee(node: HTMLElement): { destroy: () => void } {
  const update = (): void => {
    const parent = node.parentElement;
    const visible = parent?.clientWidth ?? node.clientWidth;
    const distance = Math.min(0, visible - node.scrollWidth - 8);
    const overflowing = distance < -1;
    node.classList.toggle('is-overflowing', overflowing);
    node.style.setProperty('--dock-marquee-distance', `${distance}px`);
  };
  update();
  const observer = new ResizeObserver(update);
  observer.observe(node);
  if (node.parentElement) observer.observe(node.parentElement);
  return {
    destroy() {
      observer.disconnect();
    },
  };
}
