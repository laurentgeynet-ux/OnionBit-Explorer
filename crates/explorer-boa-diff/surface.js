var out = [];
function fd(d) {
  if (!d) return '-';
  var k = '';
  if (d.get !== undefined) k += 'get';
  if (d.set !== undefined) k += 'set';
  if (d.value !== undefined) k = typeof d.value;
  return (d.writable?1:0)+''+(d.enumerable?1:0)+''+(d.configurable?1:0)+'|'+k;
}
function walk(o, path, depth) {
  if (o === null || (typeof o !== 'object' && typeof o !== 'function') || depth > 2) return;
  var names;
  try { names = Object.getOwnPropertyNames(o); } catch(e) { return; }
  for (var i = 0; i < names.length; i++) {
    var n = names[i], d;
    try { d = Object.getOwnPropertyDescriptor(o, n); } catch(e) { continue; }
    out.push(path + '.' + n + ' ' + fd(d) +
      (n === 'length' && d && typeof d.value === 'number' ? ':' + d.value : ''));
    if (depth < 2 && d && d.value !== undefined && n !== 'arguments' && n !== 'caller') {
      walk(d.value, path + '.' + n, depth + 1);
    }
  }
  if (typeof Object.getOwnPropertySymbols === 'function') {
    var syms = Object.getOwnPropertySymbols(o);
    for (var i = 0; i < syms.length; i++) {
      var s = syms[i], d2;
      try { d2 = Object.getOwnPropertyDescriptor(o, s); } catch(e) { continue; }
      out.push(path + '.@@' + String(s) + ' ' + fd(d2));
      if (depth < 2 && d2 && d2.value !== undefined) {
        walk(d2.value, path + '.@@' + String(s), depth + 1);
      }
    }
  }
}
walk(typeof globalThis !== 'undefined' ? globalThis : this, 'G', 0);
out.sort();
out.join('\n');
