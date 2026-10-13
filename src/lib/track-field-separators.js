/**
 * Keep a field separator at the end of its own field, but hide it when the
 * following field wraps to another row or when this field spans multiple rows.
 *
 * @param {HTMLElement} node
 */
export function hideWrappedTrailingTrackSeparators(node) {
  let frame = 0;

  const update = () => {
    frame = 0;
    const blocks = [...node.querySelectorAll(':scope > [data-track-block]')];
    blocks.forEach((block) => block.classList.remove('hide-trailing-separator'));
    const rects = blocks.map((block) => block.getBoundingClientRect());

    blocks.forEach((block, index) => {
      if (index >= rects.length - 1) return;
      const styles = getComputedStyle(block);
      const lineHeight = Number.parseFloat(styles.lineHeight)
        || Number.parseFloat(styles.fontSize) * 1.35;
      const blockSpansRows = rects[index].height > lineHeight * 1.5;
      const followingFieldWraps = rects[index + 1].top > rects[index].top + 1;
      if (blockSpansRows || followingFieldWraps) {
        block.classList.add('hide-trailing-separator');
      }
    });
  };

  const scheduleUpdate = () => {
    if (frame) cancelAnimationFrame(frame);
    frame = requestAnimationFrame(update);
  };

  const resizeObserver = new ResizeObserver(scheduleUpdate);
  resizeObserver.observe(node);
  const mutationObserver = new MutationObserver(scheduleUpdate);
  mutationObserver.observe(node, { childList: true, characterData: true, subtree: true });
  scheduleUpdate();

  return {
    destroy() {
      if (frame) cancelAnimationFrame(frame);
      resizeObserver.disconnect();
      mutationObserver.disconnect();
      node.querySelectorAll('.hide-trailing-separator').forEach((block) => {
        block.classList.remove('hide-trailing-separator');
      });
    },
  };
}
