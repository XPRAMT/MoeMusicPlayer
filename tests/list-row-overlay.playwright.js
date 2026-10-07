async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.goto('http://127.0.0.1:4173/', { waitUntil: 'domcontentloaded' });
  const found = await page.evaluate(() => {
    const wanted = {
      '.nav-link:hover:not(:disabled):not(.active)': 'background',
      '.nav-link.active, .nav-link.active:hover:not(:disabled)': 'background',
      '.track-row:hover::before': 'opacity',
      '.track-row.selected::before, .track-row.selected:hover::before': 'opacity',
    };
    const hits = {};
    for (const sheet of document.styleSheets) {
      let rules;
      try { rules = sheet.cssRules; } catch { continue; }
      for (const rule of rules) {
        if (!(rule instanceof CSSStyleRule)) continue;
        if (wanted[rule.selectorText]) hits[rule.selectorText] = rule.style[wanted[rule.selectorText]];
      }
    }
    return hits;
  });
  assert(found['.nav-link:hover:not(:disabled):not(.active)'] === 'rgba(var(--text-rgb), 0.1)', JSON.stringify(found));
  assert(found['.nav-link.active, .nav-link.active:hover:not(:disabled)'] === 'rgba(var(--text-rgb), 0.2)', JSON.stringify(found));
  assert(found['.track-row:hover::before'] === '0.1', JSON.stringify(found));
  assert(found['.track-row.selected::before, .track-row.selected:hover::before'] === '0.2', JSON.stringify(found));
  return { result: 'PASS', found };
}
