// Acid3 — sous-ensemble ECMAScript pur (tests 81 à 96).
// Source : wpt/acid/acid3/test.html (web-platform-tests), corps
// verbatim — extraction JS-C36, harnais informatif. Les tests
// DOM/CSS/HTTP (0-80, 97-99) sont exclus du périmètre moteur.
var __acid3 = { pass: 0, fail: [] };
function fail(m) { throw { message: m }; }
function assert(c, m) { if (!c) fail(m); }
function assertEquals(a, v, m) { if (a !== v) fail(m + " (got " + a + ", expected " + v + ")"); }
function T(n, f) { try { f(); __acid3.pass += 1; } catch (e) { __acid3.fail.push(n + ": " + (e && e.message ? e.message : e)); } }
// Préambule de la page (script inline acid3 — verbatime).
var nullInRegexpArgumentResult = 0 < /script/.test('\0script') ? "passed" : "failed";

T("t81-elisions-end", function () {
  var t1 = [,];
  var t2 = [,,];
  assertEquals(t1.length, 1, "[,] doesn't have length 1");
  assertEquals(t2.length, 2, "[,,] doesn't have length 2");
});
T("t82-elisions-middle", function () {
  var t3 = ['a', , 'c'];
  assertEquals(t3.length, 3, "['a',,'c'] doesn't have length 3");
  assert(0 in t3, "no 0 in t3");
  assert(!(1 in t3), "unexpected 1 in t3");
  assert(2 in t3, "no 2 in t3");
  assertEquals(t3[0], 'a', "t3[0] wrong");
  assertEquals(t3[2], 'c', "t3[2] wrong");
});
T("t83-array-methods", function () {
  var x = ['a', 'b', 'c'];
  assertEquals(x.unshift('A', 'B', 'C'), 6, "array.unshift() returned the wrong value");
  var s = x.join(undefined);
  assertEquals(s, 'A,B,C,a,b,c', "array.join(undefined) used wrong separator");
});
T("t84-numbers-to-strings", function () {
  assertEquals((0.0).toFixed(4), "0.0000", "toFixed(4) wrong for 0");
  assertEquals((-0.0).toFixed(4), "0.0000", "toFixed(4) wrong for -0");
  assertEquals((0.00006).toFixed(4), "0.0001", "toFixed(4) wrong for 0.00006");
  assertEquals((-0.00006).toFixed(4), "-0.0001", "toFixed(4) wrong for -0.00006");
  assertEquals((0.0).toExponential(4), "0.0000e+0", "toExponential(4) wrong for 0");
  assertEquals((-0.0).toExponential(4), "0.0000e+0", "toExponential(4) wrong for -0");
  var x = 7e-4;
  assertEquals(x.toPrecision(undefined), x.toString(undefined), "toPrecision(undefined) was wrong");
});
T("t85-substr-negative", function () {
  assertEquals("scathing".substr(-7, 3), "cat", "substr() wrong with negative numbers");
});
T("t86-date-no-args", function () {
  var d = new Date();
  assert(isNaN(d.setMilliseconds()), "calling setMilliseconds() with no arguments didn't result in NaN");
  assert(isNaN(d), "date wasn't made NaN");
  assert(isNaN(d.getDay()), "date wasn't made NaN");
});
T("t87-date-years", function () {
  var d1 = new Date(Date.UTC(99.9, 6));
  assertEquals(d1.getUTCFullYear(), 1999, "Date.UTC() didn't do proper 1900 year offsetting");
  var d2 = new Date(98.9, 6);
  assertEquals(d2.getFullYear(), 1998, "new Date() didn't do proper 1900 year offsetting");
});
T("t88-unicode-escape-ident", function () {
  var ok = false;
  try {
    eval("var test = { };\ntest.i= 0;\ntest.i\\u002b= 1;\ntest.i;\n");
  } catch (e) {
    ok = true;
  }
  assert(ok, "\\u002b was not considered a parse error in script");
});
T("t89-regexp-empty-class", function () {
  var ok = true;
  try {
    eval("/TA[])]/.exec('TA]')");
    ok = false;
  } catch (e) { }
  assert(ok, "orphaned bracket not considered parse error in regular expression literal");
  try {
    if (eval("/[]/.exec('')"))
      ok = false;
  } catch (e) {
    ok = false;
  }
  assert(ok, "/[]/ either failed to parse or matched something");
});
T("t90-regexp-backrefs", function () {
  assert(!(/(1)\0(2)/.test("12")), "NUL in regexp incorrectly ignored");
  assert((/(1)\0(2)/.test("1" + "\0" + "2")), "NUL in regexp didn't match correctly");
  assert(!(/(1)\0(2)/.test("1\02")), "octal 2 unexpectedly matched NUL");
  assertEquals(nullInRegexpArgumentResult, "passed", "failed //.test() check");
  var x = /(\3)(\1)(a)/.exec('cat');
  assert(x, "/(\\3)(\\1)(a)/ failed to match 'cat'");
  assertEquals(x.length, 4, "/(\\3)(\\1)(a)/ failed to return four components");
  assertEquals(x[0], "a", "/(\\3)(\\1)(a)/ failed to find 'a' in 'cat'");
  assert(x[1] === "", "/(\\3)(\\1)(a)/ failed to find '' in 'cat' as first part");
  assert(x[2] === "", "/(\\3)(\\1)(a)/ failed to find '' in 'cat' as second part");
  assertEquals(x[3], "a", "/(\\3)(\\1)(a)/ failed to find 'a' in 'cat' as third part");
  x = /(?!(text))(te.t)/.exec("text testing");
  assertEquals(x.length, 3, "negative lookahead test failed to return the right number of bits");
  assertEquals(x[0], "test", "negative lookahead test failed to find the right text");
  assert(x[1] === undefined, "negative lookahead test failed to return undefined for negative lookahead capture");
  assert(x[2] === "test", "negative lookahead test failed to find the right second capture");
});
T("t91-enumerable-by-default", function () {
  var test = {
    constructor: function() { return 1; },
    toString: function() { return 2; },
    toLocaleString: function() { return 3; },
    valueOf: function() { return 4; },
    hasOwnProperty: function() { return 5; },
    isPrototypeOf: function() { return 6; },
    propertyIsEnumerable: function() { return 7; },
    prototype: function() { return 8; },
    length: function() { return 9; },
    unique: function() { return 10; }
  };
  var results = [];
  for (var property in test)
    results.push([test[property](), property]);
  results.sort(function(a, b) {
    if (a[0] < b[0]) return -1;
    if (a[0] > b[0]) return 1;
    return 0;
  });
  assertEquals(results.length, 10, "missing properties");
  for (var index = 0; index < 10; index += 1)
    assertEquals(results[index][0], index+1, "order wrong at results["+index+"] == ");
  var index = 0;
  assertEquals(results[index++][1], "constructor", "failed to find constructor in expected position");
  assertEquals(results[index++][1], "toString", "failed to find toString in expected position");
  assertEquals(results[index++][1], "toLocaleString", "failed to find toLocaleString in expected position");
  assertEquals(results[index++][1], "valueOf", "failed to find valueOf in expected position");
  assertEquals(results[index++][1], "hasOwnProperty", "failed to find hasOwnProperty in expected position");
  assertEquals(results[index++][1], "isPrototypeOf", "failed to find isPrototypeOf in expected position");
  assertEquals(results[index++][1], "propertyIsEnumerable", "failed to find propertyIsEnumerable in expected position");
  assertEquals(results[index++][1], "prototype", "failed to find prototype in expected position");
  assertEquals(results[index++][1], "length", "failed to find length in expected position");
  assertEquals(results[index++][1], "unique", "failed to find unique in expected position");
});
T("t92-function-internals", function () {
  var f1 = function () { 1 };
  f1.prototype.constructor = "hello world";
  var f1i = new f1();
  assert(f1i.constructor === "hello world", "Function object's prototype's constructor was ReadOnly");
  var f2 = function () { 2 };
  var f2i = new f2();
  var count = 0;
  for (var property in f2i) {
    assert(property != "constructor", "Function object's prototype's constructor was not DontEnum");
    count += 1;
  }
  assertEquals(count, 0, "Function object had unexpected properties");
  var f3 = function (a, b) { 3 };
  delete f3.prototype.constructor;
  var f3i = new f3();
  assertEquals(f3i.constructor, Object.prototype.constructor, "Function object's prototype's constructor was DontDelete (or got magically replaced)");
});
T("t93-functionexpr-scope", function () {
  var functest;
  var vartest = 0;
  var value = (function functest(arg) {
    if (arg)
      return 1;
    vartest = 1;
    functest = function (arg) { return 2; };
    return functest(true);
  })(false);
  assertEquals(vartest, 1, "rules in 10.1.4 not followed in FunctionBody");
  assertEquals(value, 1, "semantics of FunctionExpression: function Identifier ... not followed");
  assert(!functest, "Property in step 4 of FunctionExpression: function Identifier ... leaked to parent scope");
});
T("t94-exception-scope", function () {
  var test = 'pass';
  try {
    throw 'fail';
  } catch (test) {
    test += 'ing';
  }
  assertEquals(test, 'pass', 'outer scope poisoned by exception catch{} block');
});
T("t95-expr-types", function () {
  var a = []; var s;
  s = a.length = "2147483648";
  assertEquals(typeof s, "string", "type of |\"2147483648\"| is not string");
});
T("t96-encode-uri-nul", function () {
  assertEquals(encodeURIComponent(String.fromCharCode(0)), '%00', "encodeURIComponent failed to encode U+0000");
  assertEquals(encodeURI(String.fromCharCode(0)), '%00', "encodeURI failed to encode U+0000");
});
"acid3: " + __acid3.pass + "/16 passed" + (__acid3.fail.length ? " — fails: " + __acid3.fail.join(" | ") : "");
