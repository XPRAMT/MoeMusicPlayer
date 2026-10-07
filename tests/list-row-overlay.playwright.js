async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.goto('http://127.0.0.1:4173/', { waitUntil: 'domcontentloaded' });
  const found = await page.evaluate(() => {
    const wanted = [
      '.nav-link.active, .nav-link.active:hover:not(:disabled)',
      '.track-row:hover',
      '.track-row.selected, .track-row.selected:hover',
    ];
    const hits = {};
    for (const sheet of document.styleSheets) {
      let rules;
      try { rules = sheet.cssRules; } catch { continue; }
      for (const rule of rules) {
        if (!(rule instanceof CSSStyleRule)) continue;
        if (wanted.includes(rule.selectorText)) hits[rule.selectorText] = rule.style.backgroundColor;
      }
    }
    return hits;
  });
  assert(found['.nav-link.active, .nav-link.active:hover:not(:disabled)'] === 'rgba(255, 255, 255, 0.2)', JSON.stringify(found));
  assert(found['.track-row:hover'] === 'rgba(255, 255, 255, 0.1)', JSON.stringify(found));
  assert(found['.track-row.selected, .track-row.selected:hover'] === 'rgba(255, 255, 255, 0.3)', JSON.stringify(found));
  return { result: 'PASS', found };
}
