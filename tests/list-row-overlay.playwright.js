async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.goto('http://127.0.0.1:4173/', { waitUntil: 'domcontentloaded' });
  const found = await page.evaluate(() => {
    const wanted = {
      '.nav-link:hover:not(:disabled):not(.active)': 'background',
      '.nav-link.active, .nav-link.active:hover:not(:disabled)': 'background',
      '.track-row:hover::before': 'opacity',
      '.track-row.selected::before, .track-row.selected:hover::before, .track-row.selected.active::before': 'opacity',
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
  assert(found['.track-row.selected::before, .track-row.selected:hover::before, .track-row.selected.active::before'] === '0.2', JSON.stringify(found));
  const accentRow = await page.evaluate(() => {
    for (const sheet of document.styleSheets) {
      let rules;
      try { rules = sheet.cssRules; } catch { continue; }
      for (const rule of rules) {
        if (!(rule instanceof CSSStyleRule)) continue;
        if (rule.selectorText.includes('paged-virtual-row') && rule.selectorText.includes('.active') && rule.style.background.includes('--accent')) {
          return rule.cssText;
        }
      }
    }
    return '';
  });
  assert(accentRow === '', accentRow);
  const surfaces = await page.evaluate(() => {
    const header = document.querySelector('.track-list-header, .track-table-head');
    const tree = document.querySelector('.playlist-tree');
    const item = document.querySelector('.playlist-tree-item:not(.selected)');
    const rules = [];
    for (const sheet of document.styleSheets) {
      let cssRules;
      try { cssRules = sheet.cssRules; } catch { continue; }
      for (const rule of cssRules) {
        if (!(rule instanceof CSSStyleRule)) continue;
        if (rule.selectorText.includes('track-list-header') || rule.selectorText === '.track-table-head') {
          rules.push(`${rule.selectorText} ${rule.style.background}`);
        }
      }
    }
    return {
      header: header ? getComputedStyle(header).backgroundColor : null,
      headerRules: rules,
      tree: tree ? getComputedStyle(tree).backgroundColor : null,
      unselectedItemOpacity: item ? getComputedStyle(item, '::before').opacity : null,
    };
  });
  assert(surfaces.headerRules.some((rule) => rule.includes('rgba(var(--text-rgb), 0.2)')), JSON.stringify(surfaces));
  assert(!surfaces.headerRules.some((rule) => rule.includes('0.14')), JSON.stringify(surfaces));
  assert(surfaces.tree === 'rgba(0, 0, 0, 0)', JSON.stringify(surfaces));
  if (surfaces.unselectedItemOpacity !== null) assert(surfaces.unselectedItemOpacity === '0', JSON.stringify(surfaces));
  return { result: 'PASS', found, surfaces };
}
