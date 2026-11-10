// airslice-history-card -- GENERATED from card/src by `npm run build`. Edit the source and rebuild.
var __defProp = Object.defineProperty;
var __defNormalProp = (obj, key, value) => key in obj ? __defProp(obj, key, { enumerable: true, configurable: true, writable: true, value }) : obj[key] = value;
var __publicField = (obj, key, value) => __defNormalProp(obj, typeof key !== "symbol" ? key + "" : key, value);

// ../../../../data/home/.cache/chargesteer-frontend/node_modules/@lit/reactive-element/css-tag.js
var t = globalThis;
var e = t.ShadowRoot && (void 0 === t.ShadyCSS || t.ShadyCSS.nativeShadow) && "adoptedStyleSheets" in Document.prototype && "replace" in CSSStyleSheet.prototype;
var s = /* @__PURE__ */ Symbol();
var o = /* @__PURE__ */ new WeakMap();
var n = class {
  constructor(t4, e4, o5) {
    if (this._$cssResult$ = true, o5 !== s) throw Error("CSSResult is not constructable. Use `unsafeCSS` or `css` instead.");
    this.cssText = t4, this.t = e4;
  }
  get styleSheet() {
    let t4 = this.o;
    const s4 = this.t;
    if (e && void 0 === t4) {
      const e4 = void 0 !== s4 && 1 === s4.length;
      e4 && (t4 = o.get(s4)), void 0 === t4 && ((this.o = t4 = new CSSStyleSheet()).replaceSync(this.cssText), e4 && o.set(s4, t4));
    }
    return t4;
  }
  toString() {
    return this.cssText;
  }
};
var r = (t4) => new n("string" == typeof t4 ? t4 : t4 + "", void 0, s);
var i = (t4, ...e4) => {
  const o5 = 1 === t4.length ? t4[0] : e4.reduce((e5, s4, o6) => e5 + ((t5) => {
    if (true === t5._$cssResult$) return t5.cssText;
    if ("number" == typeof t5) return t5;
    throw Error("Value passed to 'css' function must be a 'css' function result: " + t5 + ". Use 'unsafeCSS' to pass non-literal values, but take care to ensure page security.");
  })(s4) + t4[o6 + 1], t4[0]);
  return new n(o5, t4, s);
};
var S = (s4, o5) => {
  if (e) s4.adoptedStyleSheets = o5.map((t4) => t4 instanceof CSSStyleSheet ? t4 : t4.styleSheet);
  else for (const e4 of o5) {
    const o6 = document.createElement("style"), n4 = t.litNonce;
    void 0 !== n4 && o6.setAttribute("nonce", n4), o6.textContent = e4.cssText, s4.appendChild(o6);
  }
};
var c = e ? (t4) => t4 : (t4) => t4 instanceof CSSStyleSheet ? ((t5) => {
  let e4 = "";
  for (const s4 of t5.cssRules) e4 += s4.cssText;
  return r(e4);
})(t4) : t4;

// ../../../../data/home/.cache/chargesteer-frontend/node_modules/@lit/reactive-element/reactive-element.js
var { is: i2, defineProperty: e2, getOwnPropertyDescriptor: h, getOwnPropertyNames: r2, getOwnPropertySymbols: o2, getPrototypeOf: n2 } = Object;
var a = globalThis;
var c2 = a.trustedTypes;
var l = c2 ? c2.emptyScript : "";
var p = a.reactiveElementPolyfillSupport;
var d = (t4, s4) => t4;
var u = { toAttribute(t4, s4) {
  switch (s4) {
    case Boolean:
      t4 = t4 ? l : null;
      break;
    case Object:
    case Array:
      t4 = null == t4 ? t4 : JSON.stringify(t4);
  }
  return t4;
}, fromAttribute(t4, s4) {
  let i5 = t4;
  switch (s4) {
    case Boolean:
      i5 = null !== t4;
      break;
    case Number:
      i5 = null === t4 ? null : Number(t4);
      break;
    case Object:
    case Array:
      try {
        i5 = JSON.parse(t4);
      } catch (t5) {
        i5 = null;
      }
  }
  return i5;
} };
var f = (t4, s4) => !i2(t4, s4);
var b = { attribute: true, type: String, converter: u, reflect: false, useDefault: false, hasChanged: f };
Symbol.metadata ??= /* @__PURE__ */ Symbol("metadata"), a.litPropertyMetadata ??= /* @__PURE__ */ new WeakMap();
var y = class extends HTMLElement {
  static addInitializer(t4) {
    this._$Ei(), (this.l ??= []).push(t4);
  }
  static get observedAttributes() {
    return this.finalize(), this._$Eh && [...this._$Eh.keys()];
  }
  static createProperty(t4, s4 = b) {
    if (s4.state && (s4.attribute = false), this._$Ei(), this.prototype.hasOwnProperty(t4) && ((s4 = Object.create(s4)).wrapped = true), this.elementProperties.set(t4, s4), !s4.noAccessor) {
      const i5 = /* @__PURE__ */ Symbol(), h3 = this.getPropertyDescriptor(t4, i5, s4);
      void 0 !== h3 && e2(this.prototype, t4, h3);
    }
  }
  static getPropertyDescriptor(t4, s4, i5) {
    const { get: e4, set: r4 } = h(this.prototype, t4) ?? { get() {
      return this[s4];
    }, set(t5) {
      this[s4] = t5;
    } };
    return { get: e4, set(s5) {
      const h3 = e4?.call(this);
      r4?.call(this, s5), this.requestUpdate(t4, h3, i5);
    }, configurable: true, enumerable: true };
  }
  static getPropertyOptions(t4) {
    return this.elementProperties.get(t4) ?? b;
  }
  static _$Ei() {
    if (this.hasOwnProperty(d("elementProperties"))) return;
    const t4 = n2(this);
    t4.finalize(), void 0 !== t4.l && (this.l = [...t4.l]), this.elementProperties = new Map(t4.elementProperties);
  }
  static finalize() {
    if (this.hasOwnProperty(d("finalized"))) return;
    if (this.finalized = true, this._$Ei(), this.hasOwnProperty(d("properties"))) {
      const t5 = this.properties, s4 = [...r2(t5), ...o2(t5)];
      for (const i5 of s4) this.createProperty(i5, t5[i5]);
    }
    const t4 = this[Symbol.metadata];
    if (null !== t4) {
      const s4 = litPropertyMetadata.get(t4);
      if (void 0 !== s4) for (const [t5, i5] of s4) this.elementProperties.set(t5, i5);
    }
    this._$Eh = /* @__PURE__ */ new Map();
    for (const [t5, s4] of this.elementProperties) {
      const i5 = this._$Eu(t5, s4);
      void 0 !== i5 && this._$Eh.set(i5, t5);
    }
    this.elementStyles = this.finalizeStyles(this.styles);
  }
  static finalizeStyles(s4) {
    const i5 = [];
    if (Array.isArray(s4)) {
      const e4 = new Set(s4.flat(1 / 0).reverse());
      for (const s5 of e4) i5.unshift(c(s5));
    } else void 0 !== s4 && i5.push(c(s4));
    return i5;
  }
  static _$Eu(t4, s4) {
    const i5 = s4.attribute;
    return false === i5 ? void 0 : "string" == typeof i5 ? i5 : "string" == typeof t4 ? t4.toLowerCase() : void 0;
  }
  constructor() {
    super(), this._$Ep = void 0, this.isUpdatePending = false, this.hasUpdated = false, this._$Em = null, this._$Ev();
  }
  _$Ev() {
    this._$ES = new Promise((t4) => this.enableUpdating = t4), this._$AL = /* @__PURE__ */ new Map(), this._$E_(), this.requestUpdate(), this.constructor.l?.forEach((t4) => t4(this));
  }
  addController(t4) {
    (this._$EO ??= /* @__PURE__ */ new Set()).add(t4), void 0 !== this.renderRoot && this.isConnected && t4.hostConnected?.();
  }
  removeController(t4) {
    this._$EO?.delete(t4);
  }
  _$E_() {
    const t4 = /* @__PURE__ */ new Map(), s4 = this.constructor.elementProperties;
    for (const i5 of s4.keys()) this.hasOwnProperty(i5) && (t4.set(i5, this[i5]), delete this[i5]);
    t4.size > 0 && (this._$Ep = t4);
  }
  createRenderRoot() {
    const t4 = this.shadowRoot ?? this.attachShadow(this.constructor.shadowRootOptions);
    return S(t4, this.constructor.elementStyles), t4;
  }
  connectedCallback() {
    this.renderRoot ??= this.createRenderRoot(), this.enableUpdating(true), this._$EO?.forEach((t4) => t4.hostConnected?.());
  }
  enableUpdating(t4) {
  }
  disconnectedCallback() {
    this._$EO?.forEach((t4) => t4.hostDisconnected?.());
  }
  attributeChangedCallback(t4, s4, i5) {
    this._$AK(t4, i5);
  }
  _$ET(t4, s4) {
    const i5 = this.constructor.elementProperties.get(t4), e4 = this.constructor._$Eu(t4, i5);
    if (void 0 !== e4 && true === i5.reflect) {
      const h3 = (void 0 !== i5.converter?.toAttribute ? i5.converter : u).toAttribute(s4, i5.type);
      this._$Em = t4, null == h3 ? this.removeAttribute(e4) : this.setAttribute(e4, h3), this._$Em = null;
    }
  }
  _$AK(t4, s4) {
    const i5 = this.constructor, e4 = i5._$Eh.get(t4);
    if (void 0 !== e4 && this._$Em !== e4) {
      const t5 = i5.getPropertyOptions(e4), h3 = "function" == typeof t5.converter ? { fromAttribute: t5.converter } : void 0 !== t5.converter?.fromAttribute ? t5.converter : u;
      this._$Em = e4;
      const r4 = h3.fromAttribute(s4, t5.type);
      this[e4] = r4 ?? this._$Ej?.get(e4) ?? r4, this._$Em = null;
    }
  }
  requestUpdate(t4, s4, i5, e4 = false, h3) {
    if (void 0 !== t4) {
      const r4 = this.constructor;
      if (false === e4 && (h3 = this[t4]), i5 ??= r4.getPropertyOptions(t4), !((i5.hasChanged ?? f)(h3, s4) || i5.useDefault && i5.reflect && h3 === this._$Ej?.get(t4) && !this.hasAttribute(r4._$Eu(t4, i5)))) return;
      this.C(t4, s4, i5);
    }
    false === this.isUpdatePending && (this._$ES = this._$EP());
  }
  C(t4, s4, { useDefault: i5, reflect: e4, wrapped: h3 }, r4) {
    i5 && !(this._$Ej ??= /* @__PURE__ */ new Map()).has(t4) && (this._$Ej.set(t4, r4 ?? s4 ?? this[t4]), true !== h3 || void 0 !== r4) || (this._$AL.has(t4) || (this.hasUpdated || i5 || (s4 = void 0), this._$AL.set(t4, s4)), true === e4 && this._$Em !== t4 && (this._$Eq ??= /* @__PURE__ */ new Set()).add(t4));
  }
  async _$EP() {
    this.isUpdatePending = true;
    try {
      await this._$ES;
    } catch (t5) {
      Promise.reject(t5);
    }
    const t4 = this.scheduleUpdate();
    return null != t4 && await t4, !this.isUpdatePending;
  }
  scheduleUpdate() {
    return this.performUpdate();
  }
  performUpdate() {
    if (!this.isUpdatePending) return;
    if (!this.hasUpdated) {
      if (this.renderRoot ??= this.createRenderRoot(), this._$Ep) {
        for (const [t6, s5] of this._$Ep) this[t6] = s5;
        this._$Ep = void 0;
      }
      const t5 = this.constructor.elementProperties;
      if (t5.size > 0) for (const [s5, i5] of t5) {
        const { wrapped: t6 } = i5, e4 = this[s5];
        true !== t6 || this._$AL.has(s5) || void 0 === e4 || this.C(s5, void 0, i5, e4);
      }
    }
    let t4 = false;
    const s4 = this._$AL;
    try {
      t4 = this.shouldUpdate(s4), t4 ? (this.willUpdate(s4), this._$EO?.forEach((t5) => t5.hostUpdate?.()), this.update(s4)) : this._$EM();
    } catch (s5) {
      throw t4 = false, this._$EM(), s5;
    }
    t4 && this._$AE(s4);
  }
  willUpdate(t4) {
  }
  _$AE(t4) {
    this._$EO?.forEach((t5) => t5.hostUpdated?.()), this.hasUpdated || (this.hasUpdated = true, this.firstUpdated(t4)), this.updated(t4);
  }
  _$EM() {
    this._$AL = /* @__PURE__ */ new Map(), this.isUpdatePending = false;
  }
  get updateComplete() {
    return this.getUpdateComplete();
  }
  getUpdateComplete() {
    return this._$ES;
  }
  shouldUpdate(t4) {
    return true;
  }
  update(t4) {
    this._$Eq &&= this._$Eq.forEach((t5) => this._$ET(t5, this[t5])), this._$EM();
  }
  updated(t4) {
  }
  firstUpdated(t4) {
  }
};
y.elementStyles = [], y.shadowRootOptions = { mode: "open" }, y[d("elementProperties")] = /* @__PURE__ */ new Map(), y[d("finalized")] = /* @__PURE__ */ new Map(), p?.({ ReactiveElement: y }), (a.reactiveElementVersions ??= []).push("2.1.2");

// ../../../../data/home/.cache/chargesteer-frontend/node_modules/lit-html/lit-html.js
var t2 = globalThis;
var i3 = (t4) => t4;
var s2 = t2.trustedTypes;
var e3 = s2 ? s2.createPolicy("lit-html", { createHTML: (t4) => t4 }) : void 0;
var h2 = "$lit$";
var o3 = `lit$${Math.random().toFixed(9).slice(2)}$`;
var n3 = "?" + o3;
var r3 = `<${n3}>`;
var l2 = document;
var c3 = () => l2.createComment("");
var a2 = (t4) => null === t4 || "object" != typeof t4 && "function" != typeof t4;
var u2 = Array.isArray;
var d2 = (t4) => u2(t4) || "function" == typeof t4?.[Symbol.iterator];
var f2 = "[ 	\n\f\r]";
var v = /<(?:(!--|\/[^a-zA-Z])|(\/?[a-zA-Z][^>\s]*)|(\/?$))/g;
var _ = /-->/g;
var m = />/g;
var p2 = RegExp(`>|${f2}(?:([^\\s"'>=/]+)(${f2}*=${f2}*(?:[^ 	
\f\r"'\`<>=]|("|')|))|$)`, "g");
var g = /'/g;
var $ = /"/g;
var y2 = /^(?:script|style|textarea|title)$/i;
var x = (t4) => (i5, ...s4) => ({ _$litType$: t4, strings: i5, values: s4 });
var b2 = x(1);
var w = x(2);
var T = x(3);
var E = /* @__PURE__ */ Symbol.for("lit-noChange");
var A = /* @__PURE__ */ Symbol.for("lit-nothing");
var C = /* @__PURE__ */ new WeakMap();
var P = l2.createTreeWalker(l2, 129);
function V(t4, i5) {
  if (!u2(t4) || !t4.hasOwnProperty("raw")) throw Error("invalid template strings array");
  return void 0 !== e3 ? e3.createHTML(i5) : i5;
}
var N = (t4, i5) => {
  const s4 = t4.length - 1, e4 = [];
  let n4, l3 = 2 === i5 ? "<svg>" : 3 === i5 ? "<math>" : "", c4 = v;
  for (let i6 = 0; i6 < s4; i6++) {
    const s5 = t4[i6];
    let a3, u3, d3 = -1, f3 = 0;
    for (; f3 < s5.length && (c4.lastIndex = f3, u3 = c4.exec(s5), null !== u3); ) f3 = c4.lastIndex, c4 === v ? "!--" === u3[1] ? c4 = _ : void 0 !== u3[1] ? c4 = m : void 0 !== u3[2] ? (y2.test(u3[2]) && (n4 = RegExp("</" + u3[2], "g")), c4 = p2) : void 0 !== u3[3] && (c4 = p2) : c4 === p2 ? ">" === u3[0] ? (c4 = n4 ?? v, d3 = -1) : void 0 === u3[1] ? d3 = -2 : (d3 = c4.lastIndex - u3[2].length, a3 = u3[1], c4 = void 0 === u3[3] ? p2 : '"' === u3[3] ? $ : g) : c4 === $ || c4 === g ? c4 = p2 : c4 === _ || c4 === m ? c4 = v : (c4 = p2, n4 = void 0);
    const x2 = c4 === p2 && t4[i6 + 1].startsWith("/>") ? " " : "";
    l3 += c4 === v ? s5 + r3 : d3 >= 0 ? (e4.push(a3), s5.slice(0, d3) + h2 + s5.slice(d3) + o3 + x2) : s5 + o3 + (-2 === d3 ? i6 : x2);
  }
  return [V(t4, l3 + (t4[s4] || "<?>") + (2 === i5 ? "</svg>" : 3 === i5 ? "</math>" : "")), e4];
};
var S2 = class _S {
  constructor({ strings: t4, _$litType$: i5 }, e4) {
    let r4;
    this.parts = [];
    let l3 = 0, a3 = 0;
    const u3 = t4.length - 1, d3 = this.parts, [f3, v2] = N(t4, i5);
    if (this.el = _S.createElement(f3, e4), P.currentNode = this.el.content, 2 === i5 || 3 === i5) {
      const t5 = this.el.content.firstChild;
      t5.replaceWith(...t5.childNodes);
    }
    for (; null !== (r4 = P.nextNode()) && d3.length < u3; ) {
      if (1 === r4.nodeType) {
        if (r4.hasAttributes()) for (const t5 of r4.getAttributeNames()) if (t5.endsWith(h2)) {
          const i6 = v2[a3++], s4 = r4.getAttribute(t5).split(o3), e5 = /([.?@])?(.*)/.exec(i6);
          d3.push({ type: 1, index: l3, name: e5[2], strings: s4, ctor: "." === e5[1] ? I : "?" === e5[1] ? L : "@" === e5[1] ? z : H }), r4.removeAttribute(t5);
        } else t5.startsWith(o3) && (d3.push({ type: 6, index: l3 }), r4.removeAttribute(t5));
        if (y2.test(r4.tagName)) {
          const t5 = r4.textContent.split(o3), i6 = t5.length - 1;
          if (i6 > 0) {
            r4.textContent = s2 ? s2.emptyScript : "";
            for (let s4 = 0; s4 < i6; s4++) r4.append(t5[s4], c3()), P.nextNode(), d3.push({ type: 2, index: ++l3 });
            r4.append(t5[i6], c3());
          }
        }
      } else if (8 === r4.nodeType) if (r4.data === n3) d3.push({ type: 2, index: l3 });
      else {
        let t5 = -1;
        for (; -1 !== (t5 = r4.data.indexOf(o3, t5 + 1)); ) d3.push({ type: 7, index: l3 }), t5 += o3.length - 1;
      }
      l3++;
    }
  }
  static createElement(t4, i5) {
    const s4 = l2.createElement("template");
    return s4.innerHTML = t4, s4;
  }
};
function M(t4, i5, s4 = t4, e4) {
  if (i5 === E) return i5;
  let h3 = void 0 !== e4 ? s4._$Co?.[e4] : s4._$Cl;
  const o5 = a2(i5) ? void 0 : i5._$litDirective$;
  return h3?.constructor !== o5 && (h3?._$AO?.(false), void 0 === o5 ? h3 = void 0 : (h3 = new o5(t4), h3._$AT(t4, s4, e4)), void 0 !== e4 ? (s4._$Co ??= [])[e4] = h3 : s4._$Cl = h3), void 0 !== h3 && (i5 = M(t4, h3._$AS(t4, i5.values), h3, e4)), i5;
}
var R = class {
  constructor(t4, i5) {
    this._$AV = [], this._$AN = void 0, this._$AD = t4, this._$AM = i5;
  }
  get parentNode() {
    return this._$AM.parentNode;
  }
  get _$AU() {
    return this._$AM._$AU;
  }
  u(t4) {
    const { el: { content: i5 }, parts: s4 } = this._$AD, e4 = (t4?.creationScope ?? l2).importNode(i5, true);
    P.currentNode = e4;
    let h3 = P.nextNode(), o5 = 0, n4 = 0, r4 = s4[0];
    for (; void 0 !== r4; ) {
      if (o5 === r4.index) {
        let i6;
        2 === r4.type ? i6 = new k(h3, h3.nextSibling, this, t4) : 1 === r4.type ? i6 = new r4.ctor(h3, r4.name, r4.strings, this, t4) : 6 === r4.type && (i6 = new Z(h3, this, t4)), this._$AV.push(i6), r4 = s4[++n4];
      }
      o5 !== r4?.index && (h3 = P.nextNode(), o5++);
    }
    return P.currentNode = l2, e4;
  }
  p(t4) {
    let i5 = 0;
    for (const s4 of this._$AV) void 0 !== s4 && (void 0 !== s4.strings ? (s4._$AI(t4, s4, i5), i5 += s4.strings.length - 2) : s4._$AI(t4[i5])), i5++;
  }
};
var k = class _k {
  get _$AU() {
    return this._$AM?._$AU ?? this._$Cv;
  }
  constructor(t4, i5, s4, e4) {
    this.type = 2, this._$AH = A, this._$AN = void 0, this._$AA = t4, this._$AB = i5, this._$AM = s4, this.options = e4, this._$Cv = e4?.isConnected ?? true;
  }
  get parentNode() {
    let t4 = this._$AA.parentNode;
    const i5 = this._$AM;
    return void 0 !== i5 && 11 === t4?.nodeType && (t4 = i5.parentNode), t4;
  }
  get startNode() {
    return this._$AA;
  }
  get endNode() {
    return this._$AB;
  }
  _$AI(t4, i5 = this) {
    t4 = M(this, t4, i5), a2(t4) ? t4 === A || null == t4 || "" === t4 ? (this._$AH !== A && this._$AR(), this._$AH = A) : t4 !== this._$AH && t4 !== E && this._(t4) : void 0 !== t4._$litType$ ? this.$(t4) : void 0 !== t4.nodeType ? this.T(t4) : d2(t4) ? this.k(t4) : this._(t4);
  }
  O(t4) {
    return this._$AA.parentNode.insertBefore(t4, this._$AB);
  }
  T(t4) {
    this._$AH !== t4 && (this._$AR(), this._$AH = this.O(t4));
  }
  _(t4) {
    this._$AH !== A && a2(this._$AH) ? this._$AA.nextSibling.data = t4 : this.T(l2.createTextNode(t4)), this._$AH = t4;
  }
  $(t4) {
    const { values: i5, _$litType$: s4 } = t4, e4 = "number" == typeof s4 ? this._$AC(t4) : (void 0 === s4.el && (s4.el = S2.createElement(V(s4.h, s4.h[0]), this.options)), s4);
    if (this._$AH?._$AD === e4) this._$AH.p(i5);
    else {
      const t5 = new R(e4, this), s5 = t5.u(this.options);
      t5.p(i5), this.T(s5), this._$AH = t5;
    }
  }
  _$AC(t4) {
    let i5 = C.get(t4.strings);
    return void 0 === i5 && C.set(t4.strings, i5 = new S2(t4)), i5;
  }
  k(t4) {
    u2(this._$AH) || (this._$AH = [], this._$AR());
    const i5 = this._$AH;
    let s4, e4 = 0;
    for (const h3 of t4) e4 === i5.length ? i5.push(s4 = new _k(this.O(c3()), this.O(c3()), this, this.options)) : s4 = i5[e4], s4._$AI(h3), e4++;
    e4 < i5.length && (this._$AR(s4 && s4._$AB.nextSibling, e4), i5.length = e4);
  }
  _$AR(t4 = this._$AA.nextSibling, s4) {
    for (this._$AP?.(false, true, s4); t4 !== this._$AB; ) {
      const s5 = i3(t4).nextSibling;
      i3(t4).remove(), t4 = s5;
    }
  }
  setConnected(t4) {
    void 0 === this._$AM && (this._$Cv = t4, this._$AP?.(t4));
  }
};
var H = class {
  get tagName() {
    return this.element.tagName;
  }
  get _$AU() {
    return this._$AM._$AU;
  }
  constructor(t4, i5, s4, e4, h3) {
    this.type = 1, this._$AH = A, this._$AN = void 0, this.element = t4, this.name = i5, this._$AM = e4, this.options = h3, s4.length > 2 || "" !== s4[0] || "" !== s4[1] ? (this._$AH = Array(s4.length - 1).fill(new String()), this.strings = s4) : this._$AH = A;
  }
  _$AI(t4, i5 = this, s4, e4) {
    const h3 = this.strings;
    let o5 = false;
    if (void 0 === h3) t4 = M(this, t4, i5, 0), o5 = !a2(t4) || t4 !== this._$AH && t4 !== E, o5 && (this._$AH = t4);
    else {
      const e5 = t4;
      let n4, r4;
      for (t4 = h3[0], n4 = 0; n4 < h3.length - 1; n4++) r4 = M(this, e5[s4 + n4], i5, n4), r4 === E && (r4 = this._$AH[n4]), o5 ||= !a2(r4) || r4 !== this._$AH[n4], r4 === A ? t4 = A : t4 !== A && (t4 += (r4 ?? "") + h3[n4 + 1]), this._$AH[n4] = r4;
    }
    o5 && !e4 && this.j(t4);
  }
  j(t4) {
    t4 === A ? this.element.removeAttribute(this.name) : this.element.setAttribute(this.name, t4 ?? "");
  }
};
var I = class extends H {
  constructor() {
    super(...arguments), this.type = 3;
  }
  j(t4) {
    this.element[this.name] = t4 === A ? void 0 : t4;
  }
};
var L = class extends H {
  constructor() {
    super(...arguments), this.type = 4;
  }
  j(t4) {
    this.element.toggleAttribute(this.name, !!t4 && t4 !== A);
  }
};
var z = class extends H {
  constructor(t4, i5, s4, e4, h3) {
    super(t4, i5, s4, e4, h3), this.type = 5;
  }
  _$AI(t4, i5 = this) {
    if ((t4 = M(this, t4, i5, 0) ?? A) === E) return;
    const s4 = this._$AH, e4 = t4 === A && s4 !== A || t4.capture !== s4.capture || t4.once !== s4.once || t4.passive !== s4.passive, h3 = t4 !== A && (s4 === A || e4);
    e4 && this.element.removeEventListener(this.name, this, s4), h3 && this.element.addEventListener(this.name, this, t4), this._$AH = t4;
  }
  handleEvent(t4) {
    "function" == typeof this._$AH ? this._$AH.call(this.options?.host ?? this.element, t4) : this._$AH.handleEvent(t4);
  }
};
var Z = class {
  constructor(t4, i5, s4) {
    this.element = t4, this.type = 6, this._$AN = void 0, this._$AM = i5, this.options = s4;
  }
  get _$AU() {
    return this._$AM._$AU;
  }
  _$AI(t4) {
    M(this, t4);
  }
};
var B = t2.litHtmlPolyfillSupport;
B?.(S2, k), (t2.litHtmlVersions ??= []).push("3.3.3");
var D = (t4, i5, s4) => {
  const e4 = s4?.renderBefore ?? i5;
  let h3 = e4._$litPart$;
  if (void 0 === h3) {
    const t5 = s4?.renderBefore ?? null;
    e4._$litPart$ = h3 = new k(i5.insertBefore(c3(), t5), t5, void 0, s4 ?? {});
  }
  return h3._$AI(t4), h3;
};

// ../../../../data/home/.cache/chargesteer-frontend/node_modules/lit-element/lit-element.js
var s3 = globalThis;
var i4 = class extends y {
  constructor() {
    super(...arguments), this.renderOptions = { host: this }, this._$Do = void 0;
  }
  createRenderRoot() {
    const t4 = super.createRenderRoot();
    return this.renderOptions.renderBefore ??= t4.firstChild, t4;
  }
  update(t4) {
    const r4 = this.render();
    this.hasUpdated || (this.renderOptions.isConnected = this.isConnected), super.update(t4), this._$Do = D(r4, this.renderRoot, this.renderOptions);
  }
  connectedCallback() {
    super.connectedCallback(), this._$Do?.setConnected(true);
  }
  disconnectedCallback() {
    super.disconnectedCallback(), this._$Do?.setConnected(false);
  }
  render() {
    return E;
  }
};
i4._$litElement$ = true, i4["finalized"] = true, s3.litElementHydrateSupport?.({ LitElement: i4 });
var o4 = s3.litElementPolyfillSupport;
o4?.({ LitElement: i4 });
(s3.litElementVersions ??= []).push("4.2.2");

// src/i18n.js
var SV = {
  "Print history": "Tidigare utskrifter",
  "Search prints": "S\xF6k utskrifter",
  "Loading\u2026": "H\xE4mtar\u2026",
  "No prints yet.": "Inga utskrifter \xE4nnu.",
  "No prints match.": "Inga utskrifter matchar.",
  "Couldn't read the history: {0}": "Kunde inte l\xE4sa historiken: {0}",
  "Choose an AirSlice print sensor in the card's settings.": "V\xE4lj en utskriftssensor fr\xE5n AirSlice i kortets inst\xE4llningar.",
  "{0} isn't available. Is the AirSlice integration set up?": "{0} \xE4r inte tillg\xE4nglig. \xC4r integrationen AirSlice inst\xE4lld?",
  "Show more": "Visa fler",
  "1 print": "1 utskrift",
  "{0} prints": "{0} utskrifter",
  Finished: "Klar",
  Failed: "Misslyckades",
  Stopped: "Stoppad",
  "Unknown result": "Ok\xE4nt resultat",
  "Stopped at {0} %": "Stoppad vid {0} %",
  "Layer {0} of {1}": "Lager {0} av {1}",
  "Started outside AirSlice": "Startad utanf\xF6r AirSlice",
  Started: "Startade",
  Ended: "Slutade",
  Filament: "Filament",
  "Machine and power": "Maskin och el",
  Total: "Totalt",
  "Estimated cost": "Ber\xE4knad kostnad",
  "Purge and prime tower": "Utspolning och prime tower",
  "1 filament change": "1 filamentbyte",
  "{0} filament changes": "{0} filamentbyten",
  Energy: "Energi",
  "Open the model page": "\xD6ppna modellsidan",
  "Show details": "Visa detaljer",
  // The card's settings
  "Print sensor": "Utskriftssensor",
  Title: "Rubrik",
  "Prints shown at first": "Utskrifter som visas f\xF6rst",
  "Show the totals": "Visa summorna",
  "The sensor AirSlice has for each printer.": "Sensorn som AirSlice har f\xF6r varje skrivare."
};
function langOf(hass) {
  return String(hass?.locale?.language || hass?.language || "en").toLowerCase().startsWith("sv") ? "sv" : "en";
}
function t3(lang, en, args = []) {
  const text = lang === "sv" ? SV[en] ?? en : en;
  return text.replace(/\{(\d+)\}/g, (_2, i5) => String(args[Number(i5)] ?? ""));
}

// src/airslice-history-card.js
var CARD = "airslice-history-card";
var PAGE = 15;
var OUTCOMES = {
  finished: { icon: "mdi:check-circle", tone: "ok", label: "Finished" },
  failed: { icon: "mdi:close-circle", tone: "bad", label: "Failed" },
  cancelled: { icon: "mdi:stop-circle", tone: "warn", label: "Stopped" },
  unknown: { icon: "mdi:help-circle", tone: "", label: "Unknown result" }
};
var number = (lang, value, digits = 0) => new Intl.NumberFormat(lang, { maximumFractionDigits: digits, minimumFractionDigits: digits }).format(value);
function grams(lang, g2) {
  return g2 >= 1e3 ? `${number(lang, g2 / 1e3, 2)} kg` : `${number(lang, g2)} g`;
}
var money = (lang, value, currency) => `${number(lang, value, 2)}${currency ? ` ${currency}` : ""}`;
function minutes(m2) {
  if (!Number.isFinite(m2) || m2 <= 0) return "";
  const rounded = Math.max(1, Math.round(m2));
  const h3 = Math.floor(rounded / 60);
  return h3 ? `${h3} h ${rounded % 60} min` : `${rounded} min`;
}
function duration(print) {
  return print.started && print.ended ? (new Date(print.ended) - new Date(print.started)) / 6e4 : NaN;
}
function when(lang, iso) {
  const d3 = iso ? new Date(iso) : null;
  return d3 && !Number.isNaN(d3.getTime()) ? d3.toLocaleString(lang, { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" }) : "";
}
function matching(prints, search) {
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  return prints.filter((p3) => words.every((w2) => p3.name.toLowerCase().includes(w2)));
}
var safeColor = (c4) => /^#[0-9a-f]{3,8}$/i.test(c4 || "") ? c4 : "var(--disabled-color)";
var httpUrl = (u3) => /^https?:\/\//i.test(u3 || "") ? u3 : null;
var AirSliceHistoryCard = class extends i4 {
  constructor() {
    super();
    this._prints = null;
    this._search = "";
    this._photos = {};
    this._broken = {};
    this._asked = /* @__PURE__ */ new Set();
    this._request = 0;
  }
  static getStubConfig(hass) {
    const entity = Object.values(hass?.entities ?? {}).find((e4) => e4.platform === "airslice" && e4.entity_id.startsWith("sensor."));
    return { entity: entity?.entity_id ?? "" };
  }
  static getConfigForm() {
    const lang = document.documentElement.lang?.startsWith("sv") ? "sv" : "en";
    const labels = { entity: "Print sensor", title: "Title", page_size: "Prints shown at first", show_summary: "Show the totals" };
    return {
      schema: [
        { name: "entity", required: true, selector: { entity: { domain: "sensor", filter: [{ integration: "airslice" }] } } },
        { name: "title", selector: { text: {} } },
        { name: "page_size", selector: { number: { min: 1, max: 200, mode: "box" } } },
        { name: "show_summary", selector: { boolean: {} } }
      ],
      computeLabel: (s4) => t3(lang, labels[s4.name] ?? s4.name),
      computeHelper: (s4) => s4.name === "entity" ? t3(lang, "The sensor AirSlice has for each printer.") : void 0
    };
  }
  setConfig(config) {
    this._config = { show_summary: true, ...config };
    this._shown = this._pageSize;
    this._seen = void 0;
  }
  get _pageSize() {
    return Number(this._config?.page_size) > 0 ? Number(this._config.page_size) : PAGE;
  }
  getCardSize() {
    return 6;
  }
  getGridOptions() {
    return { columns: 12, min_columns: 6, rows: "auto" };
  }
  // `hass` changes with every state in the house; only this sensor matters.
  shouldUpdate(changed) {
    if (changed.size !== 1 || !changed.has("hass")) return true;
    const before = changed.get("hass");
    const entity = this._config?.entity;
    return !before || before.states[entity] !== this.hass.states[entity] || langOf(before) !== langOf(this.hass);
  }
  willUpdate() {
    const state = this.hass?.states[this._config?.entity];
    if (!state) return;
    const seen = `${state.state}|${state.attributes.latest_id}|${state.attributes.latest_outcome}`;
    if (seen !== this._seen) {
      this._seen = seen;
      this._load();
    }
  }
  updated() {
    this._askPhotos();
  }
  async _load() {
    const request = ++this._request;
    try {
      const res = await this.hass.callWS({
        type: "call_service",
        domain: "airslice",
        service: "list_prints",
        service_data: { entity_id: this._config.entity },
        return_response: true
      });
      if (request !== this._request) return;
      this._prints = res.response?.prints ?? [];
      this._error = void 0;
    } catch (e4) {
      if (request === this._request) this._error = e4.message || String(e4);
    }
  }
  get _found() {
    return matching(this._prints ?? [], this._search);
  }
  /** Photos are signed links, asked for only when a row is on screen. */
  async _askPhotos() {
    for (const p3 of this._found.slice(0, this._shown)) {
      if (!p3.photo || this._asked.has(p3.photo)) continue;
      this._asked.add(p3.photo);
      try {
        const r4 = await this.hass.callWS({ type: "media_source/resolve_media", media_content_id: p3.photo });
        this._photos = { ...this._photos, [p3.photo]: r4.url };
      } catch {
      }
    }
  }
  _setSearch(e4) {
    this._search = e4.target.value;
    this._shown = this._pageSize;
  }
  _picture(print) {
    const url = print.photo && this._photos[print.photo] || httpUrl(print.cover);
    return url && !this._broken[url] ? url : null;
  }
  _imgFailed(url) {
    this._broken = { ...this._broken, [url]: true };
  }
  render() {
    if (!this._config) return A;
    const lang = langOf(this.hass);
    const state = this.hass?.states[this._config.entity];
    return b2`<ha-card>
      <div class="head">
        <div class="title">${this._config.title || t3(lang, "Print history")}</div>
        ${state && this._config.show_summary ? this._renderSummary(lang, state) : A}
      </div>
      ${this._renderBody(lang, state)}
    </ha-card>`;
  }
  _renderSummary(lang, state) {
    const a3 = state.attributes;
    const count = Number(state.state);
    if (!count) return A;
    const currency = this._prints?.find((p3) => p3.cost)?.cost.currency ?? "";
    const parts = [
      count === 1 ? t3(lang, "1 print") : t3(lang, "{0} prints", [number(lang, count)]),
      a3.hours ? `${number(lang, a3.hours)} h` : "",
      a3.grams ? grams(lang, a3.grams) : "",
      a3.cost ? money(lang, a3.cost, currency) : ""
    ];
    return b2`<div class="muted summary">${parts.filter(Boolean).join(" \xB7 ")}</div>`;
  }
  _renderBody(lang, state) {
    if (!this._config.entity) return b2`<div class="message muted">${t3(lang, "Choose an AirSlice print sensor in the card's settings.")}</div>`;
    if (!state) return b2`<div class="message muted">${t3(lang, "{0} isn't available. Is the AirSlice integration set up?", [this._config.entity])}</div>`;
    if (this._error) return b2`<div class="message error">${t3(lang, "Couldn't read the history: {0}", [this._error])}</div>`;
    if (this._prints === null) return b2`<div class="message muted">${t3(lang, "Loading\u2026")}</div>`;
    if (!this._prints.length) return b2`<div class="message muted">${t3(lang, "No prints yet.")}</div>`;
    const found = this._found;
    return b2`
      ${this._prints.length > this._pageSize ? b2`<div class="search">
            <input type="search" .value=${this._search} placeholder=${t3(lang, "Search prints")} aria-label=${t3(lang, "Search prints")} @input=${this._setSearch} />
          </div>` : A}
      ${found.length ? A : b2`<div class="message muted">${t3(lang, "No prints match.")}</div>`}
      <ul class="list">
        ${found.slice(0, this._shown).map((p3) => this._renderPrint(lang, p3))}
      </ul>
      ${found.length > this._shown ? b2`<button class="more" @click=${() => this._shown += this._pageSize}>${t3(lang, "Show more")}</button>` : A}
    `;
  }
  _renderPrint(lang, p3) {
    const outcome = OUTCOMES[p3.outcome] ?? OUTCOMES.unknown;
    const open = this._open === p3.id;
    const picture = this._picture(p3);
    const usage = (p3.usage ?? []).filter((u3) => u3.grams > 0);
    return b2`<li>
      <button class="row" aria-expanded=${open ? "true" : "false"} aria-label=${`${p3.name}, ${t3(lang, "Show details")}`} @click=${() => this._open = open ? void 0 : p3.id}>
        <span class="thumb">
          ${picture ? b2`<img src=${picture} alt="" loading="lazy" @error=${() => this._imgFailed(picture)} />` : b2`<ha-icon icon="mdi:printer-3d"></ha-icon>`}
        </span>
        <span class="main">
          <span class="name">${p3.name}</span>
          <span class="muted small">${[when(lang, p3.ended), minutes(duration(p3))].filter(Boolean).join(" \xB7 ")}</span>
          <span class="meta small">
            <span class="outcome ${outcome.tone}"><ha-icon icon=${outcome.icon}></ha-icon>${t3(lang, outcome.label)}</span>
            ${p3.weight ? b2`<span>${grams(lang, p3.weight)}</span>` : A}
            ${p3.cost ? b2`<span>${money(lang, p3.cost.total, p3.cost.currency)}</span>` : A}
            ${usage.map((u3) => b2`<span class="swatch" style="background:${safeColor(u3.color)}" title=${`${u3.slot ?? ""} ${u3.type ?? ""}`.trim()}></span>`)}
          </span>
        </span>
      </button>
      ${open ? this._renderDetails(lang, p3, picture, usage) : A}
    </li>`;
  }
  _renderDetails(lang, p3, picture, usage) {
    const stopped = p3.outcome === "failed" || p3.outcome === "cancelled";
    const waste = p3.waste && p3.waste.flush_g + p3.waste.tower_g > 0 ? p3.waste : null;
    const model = httpUrl(p3.model_url);
    const row = (label, value) => b2`<div class="fact"><span class="muted">${label}</span><span>${value}</span></div>`;
    const currency = p3.cost?.currency;
    return b2`<div class="details">
      ${picture ? b2`<img class="photo" src=${picture} alt="" @error=${() => this._imgFailed(picture)} />` : A}
      ${p3.error ? b2`<div class="error">${p3.error}</div>` : A}
      ${p3.external ? b2`<div class="muted">${t3(lang, "Started outside AirSlice")}</div>` : A}
      ${stopped && p3.progress != null ? row(t3(lang, "Stopped at {0} %", [p3.progress]), p3.total_layers ? t3(lang, "Layer {0} of {1}", [p3.layer ?? 0, p3.total_layers]) : "") : A}
      ${p3.started ? row(t3(lang, "Started"), when(lang, p3.started)) : A}
      ${p3.ended ? row(t3(lang, "Ended"), when(lang, p3.ended)) : A}
      ${usage.map((u3) => row(b2`<span class="swatch" style="background:${safeColor(u3.color)}"></span> ${[u3.slot, u3.type].filter(Boolean).join(" ")}`, grams(lang, u3.grams)))}
      ${waste ? row(
      b2`${t3(lang, "Purge and prime tower")}${waste.changes ? b2`, ${waste.changes === 1 ? t3(lang, "1 filament change") : t3(lang, "{0} filament changes", [waste.changes])}` : A}`,
      `${grams(lang, waste.flush_g + waste.tower_g)}${waste.cost != null ? ` \xB7 ${money(lang, waste.cost, currency)}` : ""}`
    ) : A}
      ${p3.cost?.filament != null ? row(t3(lang, "Filament"), money(lang, p3.cost.filament, currency)) : A}
      ${p3.cost?.machine != null ? row(t3(lang, "Machine and power"), money(lang, p3.cost.machine, currency)) : A}
      ${p3.cost ? row(t3(lang, p3.cost.actual ? "Total" : "Estimated cost"), money(lang, p3.cost.total, currency)) : A}
      ${p3.energy_kwh != null ? row(t3(lang, "Energy"), `${number(lang, p3.energy_kwh, 2)} kWh`) : A}
      ${model ? b2`<a class="link" href=${model} target="_blank" rel="noopener noreferrer">${t3(lang, "Open the model page")}</a>` : A}
    </div>`;
  }
};
__publicField(AirSliceHistoryCard, "properties", {
  hass: { attribute: false },
  _config: { state: true },
  _prints: { state: true },
  _error: { state: true },
  _search: { state: true },
  _shown: { state: true },
  _open: { state: true },
  _photos: { state: true },
  _broken: { state: true }
});
__publicField(AirSliceHistoryCard, "styles", i`
    ha-card { padding: 16px 0 8px; }
    .head { padding: 0 16px 8px; }
    .title { font-size: var(--ha-font-size-l, 18px); font-weight: 500; color: var(--primary-text-color); }
    .muted { color: var(--secondary-text-color); }
    .small, .summary { font-size: var(--ha-font-size-s, 12px); }
    .error { color: var(--error-color); }
    .message { padding: 8px 16px 16px; }
    .search { padding: 0 16px 8px; }
    input {
      width: 100%; box-sizing: border-box; padding: 10px 12px;
      background: var(--mdc-text-field-fill-color, rgba(127, 127, 127, 0.12));
      color: var(--primary-text-color); border: none;
      border-bottom: 1px solid var(--divider-color); border-radius: 4px 4px 0 0;
      font: inherit; font-size: 16px; /* smaller makes iOS zoom in on focus */
    }
    input:focus { outline: none; border-bottom-color: var(--primary-color); }
    .list { list-style: none; margin: 0; padding: 0; }
    li + li { border-top: 1px solid var(--divider-color); }
    button { font: inherit; color: inherit; background: none; border: none; cursor: pointer; }
    button:focus-visible, a:focus-visible { outline: 2px solid var(--primary-color); outline-offset: -2px; }
    .row { display: flex; gap: 12px; align-items: center; width: 100%; padding: 10px 16px; text-align: left; }
    .row:hover { background: var(--secondary-background-color); }
    .thumb {
      flex: none; width: 56px; height: 56px; border-radius: 8px; overflow: hidden;
      display: flex; align-items: center; justify-content: center;
      background: var(--secondary-background-color); color: var(--secondary-text-color);
    }
    .thumb img { width: 100%; height: 100%; object-fit: cover; }
    .main { display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
    .name { color: var(--primary-text-color); font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .meta { display: flex; flex-wrap: wrap; align-items: center; gap: 4px 10px; color: var(--secondary-text-color); }
    .outcome { display: inline-flex; align-items: center; gap: 3px; --mdc-icon-size: 16px; }
    .outcome.ok { color: var(--success-color, var(--green-color)); }
    .outcome.bad { color: var(--error-color); }
    .outcome.warn { color: var(--warning-color); }
    .swatch { display: inline-block; width: 11px; height: 11px; border-radius: 50%; box-shadow: inset 0 0 0 1px var(--divider-color); vertical-align: -1px; }
    .details { display: flex; flex-direction: column; gap: 6px; padding: 2px 16px 14px; }
    .photo { width: 100%; max-height: 320px; object-fit: contain; border-radius: 8px; background: var(--secondary-background-color); }
    .fact { display: flex; justify-content: space-between; gap: 12px; }
    .fact > :last-child { text-align: right; color: var(--primary-text-color); white-space: nowrap; }
    .link { color: var(--primary-color); padding-top: 4px; }
    .more { display: block; width: 100%; padding: 12px; color: var(--primary-color); font-weight: 500; }
  `);
if (!customElements.get(CARD)) customElements.define(CARD, AirSliceHistoryCard);
window.customCards = window.customCards || [];
if (!window.customCards.some((c4) => c4.type === CARD)) {
  window.customCards.push({ type: CARD, name: "AirSlice history", description: "Every print AirSlice has followed, with photos, filament and cost.", documentationURL: "https://github.com/thrawnarn/AirSlice" });
}
export {
  AirSliceHistoryCard,
  duration,
  grams,
  matching,
  minutes
};
/*! Bundled license information:

@lit/reactive-element/css-tag.js:
  (**
   * @license
   * Copyright 2019 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)

@lit/reactive-element/reactive-element.js:
lit-html/lit-html.js:
lit-element/lit-element.js:
  (**
   * @license
   * Copyright 2017 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)

lit-html/is-server.js:
  (**
   * @license
   * Copyright 2022 Google LLC
   * SPDX-License-Identifier: BSD-3-Clause
   *)
*/
