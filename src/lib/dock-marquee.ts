import { devicePixelEasing, watchDevicePixelRatio } from './device-pixel-steps.js';

/** Mark a text for the CSS marquee (`.dock-marquee` / `.dock-marquee-text` in app.css) only when it overflows its slot:
 * the play dock title/artist and the device names in the output channel-map matrix.
 * The scroll moves in whole device pixels (AGENTS.md device-pixel rule): the moving part of the keyframes is eased with
 * `steps()`, one step per device pixel of the distance, so the window presents only when the text moves a pixel (the
 * pauses at both ends hold still and present nothing). */
export function dockMarquee(node: HTMLElement): { destroy: () => void } {
  const update = (): void => {
    const parent = node.parentElement;
    const visible = parent?.clientWidth ?? node.clientWidth;
    const distance = Math.min(0, visible - node.scrollWidth - 8);
    const overflowing = distance < -1;
    node.classList.toggle('is-overflowing', overflowing);
    node.style.setProperty('--dock-marquee-distance', `${distance}px`);
    node.style.animationTimingFunction = overflowing ? devicePixelEasing(distance, window.devicePixelRatio || 1) : '';
  };
  update();
  const observer = new ResizeObserver(update);
  observer.observe(node);
  if (node.parentElement) observer.observe(node.parentElement);
  const stopRatio = watchDevicePixelRatio(update);
  return {
    destroy() {
      observer.disconnect();
      stopRatio();
    },
  };
}
