performance.mark("js-parse-end:t2-19741658de225445.js");
export const __rspack_esm_id="t2";export const __rspack_esm_ids=["t2"];export const __webpack_modules__={s7(t,e,o){let n;o("Fp");var r,i,s,a,l,p,c,u,d,h,f,m,v=o("NZ"),g=function(t,e,o,n){if("a"===o&&!n)throw TypeError("Private accessor was defined without a getter");if("function"==typeof e?t!==e||!n:!e.has(t))throw TypeError("Cannot read private member from an object whose class did not declare it");return"m"===o?n:"a"===o?n.call(t):n?n.value:e.get(t)},w=function(t,e,o,n,r){if("m"===n)throw TypeError("Private method is not writable");if("a"===n&&!r)throw TypeError("Private accessor was defined without a setter");if("function"==typeof e?t!==e||!r:!e.has(t))throw TypeError("Cannot write private member to an object whose class did not declare it");return"a"===n?r.call(t,o):r?r.value=o:e.set(t,o),o};let b=t=>n?t.matches(n):function(t){try{return n=":popover-open",t.matches(n)}catch{try{return n=":open",t.matches(":open")}catch{return n=".\\:popover-open",t.matches(".\\:popover-open")}}}(t),y="sr-only",E=["tooltip-n","tooltip-s","tooltip-e","tooltip-w","tooltip-ne","tooltip-se","tooltip-nw","tooltip-sw"];function T(t){for(let e of k)e!==t&&(b(e)?e.hidePopover():k.delete(e))}function A(){T()}function L(t){setTimeout(()=>{for(let e of k)b(e)&&"focus"===e.showReason&&e.control!==t.target&&e.hidePopover()},0)}let S=new Set,k=new Set;class M extends HTMLElement{constructor(){super(...arguments),r.add(this),i.set(this,void 0),s.set(this,"center"),a.set(this,"outside-bottom"),l.set(this,!1),p.set(this,"mouse"),c.set(this,!1)}styles(){return`
      :host {
        --tooltip-top: var(--tool-tip-position-top, 0);
        --tooltip-left: var(--tool-tip-position-left, 0);
        padding: var(--overlay-paddingBlock-condensed) var(--overlay-padding-condensed) !important;
        font: var(--text-body-shorthand-small);
        color: var(--tooltip-fgColor, var(--fgColor-onEmphasis)) !important;
        text-align: center;
        text-decoration: none;
        text-shadow: none;
        text-transform: none;
        letter-spacing: normal;
        word-wrap: break-word;
        white-space: pre;
        background: var(--tooltip-bgColor, var(--bgColor-emphasis)) !important;
        border-radius: var(--borderRadius-medium);
        border: 0 !important;
        opacity: 0;
        max-width: min(var(--overlay-width-small), 100vw);
        word-wrap: break-word;
        white-space: normal;
        width: max-content !important;
        inset: var(--tooltip-top) auto auto var(--tooltip-left) !important;
        overflow: visible !important;
        text-wrap: balance;
      }

      :host(:is(.tooltip-n, .tooltip-nw, .tooltip-ne)) {
        --tooltip-top: calc(var(--tool-tip-position-top, 0) - var(--overlay-offset, 0.25rem));
        --tooltip-left: var(--tool-tip-position-left);
      }

      :host(:is(.tooltip-s, .tooltip-sw, .tooltip-se)) {
        --tooltip-top: calc(var(--tool-tip-position-top, 0) + var(--overlay-offset, 0.25rem));
        --tooltip-left: var(--tool-tip-position-left);
      }

      :host(.tooltip-w) {
        --tooltip-top: var(--tool-tip-position-top);
        --tooltip-left: calc(var(--tool-tip-position-left, 0) - var(--overlay-offset, 0.25rem));
      }

      :host(.tooltip-e) {
        --tooltip-top: var(--tool-tip-position-top);
        --tooltip-left: calc(var(--tool-tip-position-left, 0) + var(--overlay-offset, 0.25rem));
      }

      :host:after{
        position: absolute;
        display: block;
        right: 0;
        left: 0;
        height: var(--overlay-offset, 0.25rem);
        content: "";
      }

      :host(.tooltip-s):after,
      :host(.tooltip-se):after,
      :host(.tooltip-sw):after {
        bottom: 100%
      }

      :host(.tooltip-n):after,
      :host(.tooltip-ne):after,
      :host(.tooltip-nw):after {
        top: 100%;
      }

      @keyframes tooltip-appear {
        from {
          opacity: 0;
        }
        to {
          opacity: 1;
        }
      }

      :host(:popover-open),
      :host(:popover-open):before {
        animation-name: tooltip-appear;
        animation-duration: .1s;
        animation-fill-mode: forwards;
        animation-timing-function: ease-in;
      }

      :host(.\\:popover-open) {
        animation-name: tooltip-appear;
        animation-duration: .1s;
        animation-fill-mode: forwards;
        animation-timing-function: ease-in;
      }

      @media (forced-colors: active) {
        :host {
          outline: solid 1px transparent;
        }

        :host:before {
          display: none;
        }
      }
    `}get showReason(){return g(this,p,"f")}get htmlFor(){return this.getAttribute("for")||""}set htmlFor(t){this.setAttribute("for",t)}get type(){return"label"===this.getAttribute("data-type")?"label":"description"}set type(t){this.setAttribute("data-type",t)}get direction(){return this.getAttribute("data-direction")||"s"}set direction(t){this.setAttribute("data-direction",t)}get control(){return this.ownerDocument.getElementById(this.htmlFor)}set hiddenFromView(t){t&&b(this)?this.hidePopover():t||b(this)||this.showPopover()}get hiddenFromView(){return!b(this)}connectedCallback(){if(S.add(this),g(this,r,"m",h).call(this),g(this,r,"m",f).call(this),!this.shadowRoot){let t=this.attachShadow({mode:"open"});t.appendChild(document.createElement("style")).textContent=this.styles(),t.appendChild(document.createElement("slot"))}g(this,r,"m",u).call(this,!1),w(this,l,!0,"f"),g(this,r,"m",d).call(this)}disconnectedCallback(){S.delete(this),k.delete(this),g(this,i,"f")?.abort()}async handleEvent(t){if(!this.control)return;let e=b(this);"beforetoggle"===t.type&&t.currentTarget!==this&&w(this,c,"open"===t.newState,"f");let o=("mouseenter"===t.type||"focus"===t.type&&(navigator.webdriver||this.control.matches(":focus-visible")))&&!g(this,c,"f"),n="mouseleave"===t.type&&t.relatedTarget!==this.control&&t.relatedTarget!==this,i="keydown"===t.type&&"Escape"===t.key,s="mousedown"===t.type&&t.currentTarget===this.control,a="beforetoggle"===t.type&&t.currentTarget!==this&&"open"===t.newState;e&&i&&(t.stopImmediatePropagation(),t.preventDefault()),await Promise.resolve(),e||!o||b(this)?e&&(n||i||s||a)&&b(this)&&this.hidePopover():(w(this,p,"mouseenter"===t.type?"mouse":"focus","f"),this.showPopover()),"toggle"===t.type&&g(this,r,"m",u).call(this,"open"===t.newState)}attributeChangedCallback(t){this.isConnected&&("for"===t?g(this,r,"m",d).call(this):"id"===t||"data-type"===t?g(this,r,"m",h).call(this):"data-direction"===t&&g(this,r,"m",f).call(this))}}i=new WeakMap,s=new WeakMap,a=new WeakMap,l=new WeakMap,p=new WeakMap,c=new WeakMap,r=new WeakSet,u=function(t){t?(k.add(this),this.classList.remove(y),T(this),g(this,r,"m",m).call(this)):(k.delete(this),this.classList.remove(...E),this.classList.add(y))},d=function(){if(!this.control)return;this.setAttribute("role","tooltip"),g(this,i,"f")?.abort(),w(this,i,new AbortController,"f");let{signal:t}=g(this,i,"f");this.addEventListener("mouseleave",this,{signal:t}),this.addEventListener("toggle",this,{signal:t}),this.control.addEventListener("mouseenter",this,{signal:t}),this.control.addEventListener("mouseleave",this,{signal:t}),this.control.addEventListener("focus",this,{signal:t}),this.control.addEventListener("mousedown",this,{signal:t}),this.control.popoverTargetElement?.addEventListener("beforetoggle",this,{signal:t}),this.ownerDocument.addEventListener("focusout",A),this.ownerDocument.addEventListener("focusin",L),this.ownerDocument.addEventListener("keydown",this,{signal:t,capture:!0})},h=function(){if(this.id&&this.control)if("label"===this.type){let t=this.control.getAttribute("aria-labelledby");t=t?t.split(" ").includes(this.id)?`${t}`:`${t} ${this.id}`:this.id,this.control.setAttribute("aria-labelledby",t),this.setAttribute("aria-hidden","true")}else{let t=this.control.getAttribute("aria-describedby");t=t?t.split(" ").includes(this.id)?`${t}`:`${t} ${this.id}`:this.id,this.control.setAttribute("aria-describedby",t)}},f=function(){this.classList.remove(...E);let t=this.direction;"n"===t?(w(this,s,"center","f"),w(this,a,"outside-top","f")):"ne"===t?(w(this,s,"end","f"),w(this,a,"outside-top","f")):"e"===t?(w(this,s,"center","f"),w(this,a,"outside-right","f")):"se"===t?(w(this,s,"end","f"),w(this,a,"outside-bottom","f")):"s"===t?(w(this,s,"center","f"),w(this,a,"outside-bottom","f")):"sw"===t?(w(this,s,"start","f"),w(this,a,"outside-bottom","f")):"w"===t?(w(this,s,"center","f"),w(this,a,"outside-left","f")):"nw"===t&&(w(this,s,"start","f"),w(this,a,"outside-top","f"))},m=function(){if(!this.control||!g(this,l,"f")||!b(this))return;let t=(0,v.uG)(this,this.control,{side:g(this,a,"f"),align:g(this,s,"f"),anchorOffset:0}),e=t.anchorSide,o=t.anchorAlign;this.style.setProperty("--tool-tip-position-top",`${t.top}px`),this.style.setProperty("--tool-tip-position-left",`${t.left}px`);let n="s";n="outside-left"===e?"w":"outside-right"===e?"e":"outside-top"===e?"center"===o?"n":"start"===o?"ne":"nw":"center"===o?"s":"start"===o?"se":"sw",this.classList.add(`tooltip-${n}`)},M.observedAttributes=["data-type","data-direction","id","for"],window.customElements.get("tool-tip")||(window.ToolTipElement=M,window.customElements.define("tool-tip",M))},NL(t,e,o){let n="data-close-dialog",r=`[${n}]`;function i(t){let e=Array.from(t.querySelectorAll("[autofocus]")).filter(a)[0];e||(e=t,t.setAttribute("tabindex","-1")),e.focus()}function s(t){let e=t.currentTarget;e instanceof Element&&("Escape"===t.key||"Esc"===t.key?(u(e,!1),t.stopPropagation()):"Tab"===t.key&&function(t){if(!(t.currentTarget instanceof Element))return;let e=t.currentTarget.querySelector("details-dialog");if(!e)return;t.preventDefault();let o=Array.from(e.querySelectorAll("*")).filter(a);if(0===o.length)return;let n=t.shiftKey?-1:1,r=e.getRootNode(),i=e.contains(r.activeElement)?r.activeElement:null,s=-1===n?-1:0;if(i instanceof HTMLElement){let t=o.indexOf(i);-1!==t&&(s=t+n)}s<0?s=o.length-1:s%=o.length,o[s].focus()}(t))}function a(t){var e;return t.tabIndex>=0&&!t.disabled&&!(e=t).hidden&&(!e.type||"hidden"!==e.type)&&(e.offsetWidth>0||e.offsetHeight>0)}function l(t){let e=t.querySelector("details-dialog");return!(e instanceof v)||e.dispatchEvent(new CustomEvent("details-dialog-close",{bubbles:!0,cancelable:!0}))}function p(t){if(!(t.currentTarget instanceof Element))return;let e=t.currentTarget.closest("details");e&&e.hasAttribute("open")&&(l(e)||(t.preventDefault(),t.stopPropagation()))}function c(t){let e=t.currentTarget;if(!(e instanceof Element))return;let o=e.querySelector("details-dialog");if(o instanceof v)if(e.hasAttribute("open")){let t="getRootNode"in o?o.getRootNode():document;t.activeElement instanceof HTMLElement&&m.set(o,{details:e,activeElement:t.activeElement}),i(o),e.addEventListener("keydown",s)}else{var n,r;let t;for(let t of o.querySelectorAll("form"))t.reset();let i=(n=e,r=o,(t=m.get(r))&&t.activeElement instanceof HTMLElement?t.activeElement:n.querySelector("summary"));i&&i.focus(),e.removeEventListener("keydown",s)}}function u(t,e){e!==t.hasAttribute("open")&&(e?t.setAttribute("open",""):l(t)&&t.removeAttribute("open"))}function d(t){let e=t.currentTarget;if(!(e instanceof Element))return;let o=e.querySelector("details-dialog");if(!(o instanceof v))return;let n=o.querySelector("include-fragment:not([src])");if(!n)return;let r=o.src;null!==r&&(n.addEventListener("loadend",()=>{e.hasAttribute("open")&&i(o)}),n.setAttribute("src",r),f(e))}function h(t,e,o){f(t),e&&t.addEventListener("toggle",d,{once:!0}),e&&o&&t.addEventListener("mouseover",d,{once:!0})}function f(t){t.removeEventListener("toggle",d),t.removeEventListener("mouseover",d)}let m=new WeakMap;class v extends HTMLElement{static get CLOSE_ATTR(){return n}static get CLOSE_SELECTOR(){return r}constructor(){super(),m.set(this,{details:null,activeElement:null}),this.addEventListener("click",function({target:t}){if(!(t instanceof Element))return;let e=t.closest("details");e&&t.closest(r)&&u(e,!1)})}get src(){return this.getAttribute("src")}set src(t){this.setAttribute("src",t||"")}get preload(){return this.hasAttribute("preload")}set preload(t){t?this.setAttribute("preload",""):this.removeAttribute("preload")}connectedCallback(){this.setAttribute("role","dialog"),this.setAttribute("aria-modal","true");let t=m.get(this);if(!t)return;let e=this.parentElement;if(!e)return;let o=e.querySelector("summary");o&&(o.hasAttribute("role")||o.setAttribute("role","button"),o.addEventListener("click",p,{capture:!0})),e.addEventListener("toggle",c),t.details=e,h(e,this.src,this.preload)}disconnectedCallback(){let t=m.get(this);if(!t)return;let{details:e}=t;if(!e)return;e.removeEventListener("toggle",c),f(e);let o=e.querySelector("summary");o&&o.removeEventListener("click",p,{capture:!0}),t.details=null}toggle(t){let e=m.get(this);if(!e)return;let{details:o}=e;o&&u(o,t)}static get observedAttributes(){return["src","preload"]}attributeChangedCallback(){let t=m.get(this);if(!t)return;let{details:e}=t;e&&h(e,this.src,this.preload)}}window.customElements.get("details-dialog")||(window.DetailsDialogElement=v,window.customElements.define("details-dialog",v)),o.d(e,{},{A:v})},W(t,e,o){function n(...t){return JSON.stringify(t,(t,e)=>"object"==typeof e?e:String(e))}function r(t,e={}){let{hash:o=n,cache:i=new Map}=e;return function(...e){let n=o.apply(this,e);if(i.has(n))return i.get(n);let r=t.apply(this,e);return r instanceof Promise&&(r=r.catch(t=>{throw i.delete(n),t})),i.set(n,r),r}}o.d(e,{A:()=>r,G:()=>n})},Fp(){var t=class extends Event{oldState;newState;constructor(t,{oldState:e="",newState:o="",...n}={}){super(t,n),this.oldState=String(e||""),this.newState=String(o||"")}},e=new WeakMap;function o(o,n,r){e.set(o,setTimeout(()=>{e.has(o)&&o.dispatchEvent(new t("toggle",{cancelable:!1,oldState:n,newState:r}))},0))}var n=globalThis.ShadowRoot||function(){},r=globalThis.HTMLDialogElement||function(){},i=new WeakMap,s=new WeakMap,a=new WeakMap;function l(t){return a.get(t)||"hidden"}var p=new WeakMap;function c(t,e){return!("auto"!==t.popover&&"manual"!==t.popover||!t.isConnected||e&&"showing"!==l(t)||!e&&"hidden"!==l(t)||t instanceof r&&t.hasAttribute("open"))&&document.fullscreenElement!==t}function u(t){return t?Array.from(s.get(t.ownerDocument)||[]).indexOf(t)+1:0}function d(t){let e=s.get(t);for(let t of e||[])if(t.isConnected)return t;else e.delete(t);return null}function h(t){return"function"==typeof t.getRootNode?t.getRootNode():t.parentNode?h(t.parentNode):t}function f(t){for(;t;){if(t instanceof HTMLElement&&"auto"===t.popover&&"showing"===a.get(t))return t;if((t=t instanceof Element&&t.assignedSlot||t.parentElement||h(t))instanceof n&&(t=t.host),t instanceof Document)return}}var m=new WeakMap;function v(e){if(!c(e,!1))return;let r=e.ownerDocument;if(!e.dispatchEvent(new t("beforetoggle",{cancelable:!0,oldState:"closed",newState:"open"}))||!c(e,!1))return;let l=!1;if("auto"===e.popover){let t=e.getAttribute("popover");if(b(function(t){let e=new Map,o=0;for(let n of s.get(t.ownerDocument)||[])e.set(n,o),o+=1;e.set(t,o),o+=1;let n=null;return!function(t){let o=f(t);if(null===o)return;let r=e.get(o);(null===n||e.get(n)<r)&&(n=o)}(t.parentElement||h(t)),n}(e)||r,!1,!0),t!==e.getAttribute("popover")||!c(e,!1))return}d(r)||(l=!0),m.delete(e);let u=r.activeElement;e.classList.add(":popover-open"),a.set(e,"showing"),i.has(r)||i.set(r,new Set),i.get(r).add(e),(function(t){if(t.shadowRoot&&!0!==t.shadowRoot.delegatesFocus)return null;let e=t;e.shadowRoot&&(e=e.shadowRoot);let o=e.querySelector("[autofocus]");if(o)return o;for(let t of e.querySelectorAll("slot"))for(let e of t.assignedElements({flatten:!0}))if(e.hasAttribute("autofocus"))return e;else if(o=e.querySelector("[autofocus]"))return o;let r=t.ownerDocument.createTreeWalker(e,NodeFilter.SHOW_ELEMENT),i=r.currentNode;for(;i;){var s;if(!((s=i).hidden||s instanceof n||(s instanceof HTMLButtonElement||s instanceof HTMLInputElement||s instanceof HTMLSelectElement||s instanceof HTMLTextAreaElement||s instanceof HTMLOptGroupElement||s instanceof HTMLOptionElement||s instanceof HTMLFieldSetElement)&&s.disabled||s instanceof HTMLInputElement&&"hidden"===s.type||s instanceof HTMLAnchorElement&&""===s.href)&&"number"==typeof s.tabIndex&&-1!==s.tabIndex)return i;i=r.nextNode()}})(e)?.focus(),"auto"===e.popover&&(s.has(r)||s.set(r,new Set),s.get(r).add(e),A(p.get(e),!0)),l&&u&&"auto"===e.popover&&m.set(e,u),o(e,"closed","open")}function g(e,n=!1,r=!1){if(!c(e,!0))return;let l=e.ownerDocument;if("auto"===e.popover&&(b(e,n,r),!c(e,!0))||(A(p.get(e),!1),p.delete(e),r&&(e.dispatchEvent(new t("beforetoggle",{oldState:"open",newState:"closed"})),!c(e,!0))))return;i.get(l)?.delete(e),s.get(l)?.delete(e),e.classList.remove(":popover-open"),a.set(e,"hidden"),r&&o(e,"open","closed");let u=m.get(e);u&&(m.delete(e),n&&u.focus())}function w(t,e=!1,o=!1){let n=d(t);for(;n;)g(n,e,o),n=d(t)}function b(t,e,o){let n=t.ownerDocument||t;if(t instanceof Document)return w(n,e,o);let r=null,i=!1;for(let e of s.get(n)||[])if(e===t)i=!0;else if(i){r=e;break}if(!i)return w(n,e,o);for(;r&&"showing"===l(r)&&s.get(n)?.size;)g(r,e,o)}var y=new WeakMap;function E(t){let e,o;if(!t.isTrusted)return;let r=t.composedPath()[0];if(!r)return;let i=r.ownerDocument;if(!d(i))return;let s=(e=f(r),o=function(t){for(;t;){let e=t.popoverTargetElement;if(e instanceof HTMLElement)return e;if((t=t.parentElement||h(t))instanceof n&&(t=t.host),t instanceof Document)return}}(r),u(e)>u(o)?e:o);if(s&&"pointerdown"===t.type)y.set(i,s);else if("pointerup"===t.type){let t=y.get(i)===s;y.delete(i),t&&b(s||i,!1,!0)}}var T=new WeakMap;function A(t,e=!1){if(!t)return;T.has(t)||T.set(t,t.getAttribute("aria-expanded"));let o=t.popoverTargetElement;if(o instanceof HTMLElement&&"auto"===o.popover)t.setAttribute("aria-expanded",String(e));else{let e=T.get(t);e?t.setAttribute("aria-expanded",e):t.removeAttribute("aria-expanded")}}var L=globalThis.ShadowRoot||function(){};function S(t,e,o){let n=t[e];Object.defineProperty(t,e,{value(t){return n.call(this,o(t))}})}var k=/(^|[^\\]):popover-open\b/g,M=null;function x(t){let e,o=(e="function"==typeof globalThis.CSSLayerBlockRule,`
${e?"@layer popover-polyfill {":""}
  :where([popover]) {
    position: fixed;
    z-index: 2147483647;
    inset: 0;
    padding: 0.25em;
    width: fit-content;
    height: fit-content;
    border-width: initial;
    border-color: initial;
    border-image: initial;
    border-style: solid;
    background-color: canvas;
    color: canvastext;
    overflow: auto;
    margin: auto;
  }

  :where([popover]:not(.\\:popover-open)) {
    display: none;
  }

  :where(dialog[popover].\\:popover-open) {
    display: block;
  }

  :where(dialog[popover][open]) {
    display: revert;
  }

  :where([anchor].\\:popover-open) {
    inset: auto;
  }

  :where([anchor]:popover-open) {
    inset: auto;
  }

  @supports not (background-color: canvas) {
    :where([popover]) {
      background-color: white;
      color: black;
    }
  }

  @supports (width: -moz-fit-content) {
    :where([popover]) {
      width: -moz-fit-content;
      height: -moz-fit-content;
    }
  }

  @supports not (inset: 0) {
    :where([popover]) {
      top: 0;
      left: 0;
      right: 0;
      bottom: 0;
    }
  }
${e?"}":""}
`);if(null===M)try{(M=new CSSStyleSheet).replaceSync(o)}catch{M=!1}if(!1===M){let e=document.createElement("style");e.textContent=o,t instanceof Document?t.head.prepend(e):t.prepend(e)}else t.adoptedStyleSheets=[M,...t.adoptedStyleSheets]}"u">typeof HTMLElement&&"object"==typeof HTMLElement.prototype&&"popover"in HTMLElement.prototype||function(){var e;if("u"<typeof window)return;function o(t){return t?.includes(":popover-open")&&(t=t.replace(k,"$1.\\:popover-open")),t}window.ToggleEvent=window.ToggleEvent||t,S(Document.prototype,"querySelector",o),S(Document.prototype,"querySelectorAll",o),S(Element.prototype,"querySelector",o),S(Element.prototype,"querySelectorAll",o),S(Element.prototype,"matches",o),S(Element.prototype,"closest",o),S(DocumentFragment.prototype,"querySelectorAll",o),Object.defineProperties(HTMLElement.prototype,{popover:{enumerable:!0,configurable:!0,get(){if(!this.hasAttribute("popover"))return null;let t=(this.getAttribute("popover")||"").toLowerCase();return""===t||"auto"==t?"auto":"manual"},set(t){null===t?this.removeAttribute("popover"):this.setAttribute("popover",t)}},showPopover:{enumerable:!0,configurable:!0,value(){v(this)}},hidePopover:{enumerable:!0,configurable:!0,value(){g(this,!0,!0)}},togglePopover:{enumerable:!0,configurable:!0,value(t){"showing"===a.get(this)&&void 0===t||!1===t?g(this,!0,!0):(void 0===t||!0===t)&&v(this)}}});let n=Element.prototype.attachShadow;n&&Object.defineProperties(Element.prototype,{attachShadow:{enumerable:!0,configurable:!0,writable:!0,value(t){let e=n.call(this,t);return x(e),e}}});let r=HTMLElement.prototype.attachInternals;r&&Object.defineProperties(HTMLElement.prototype,{attachInternals:{enumerable:!0,configurable:!0,writable:!0,value(){let t=r.call(this);return t.shadowRoot&&x(t.shadowRoot),t}}});let i=new WeakMap;function s(t){Object.defineProperties(t.prototype,{popoverTargetElement:{enumerable:!0,configurable:!0,set(t){if(null===t)this.removeAttribute("popovertarget"),i.delete(this);else if(t instanceof Element)this.setAttribute("popovertarget",""),i.set(this,t);else throw TypeError("popoverTargetElement must be an element or null")},get(){if("button"!==this.localName&&"input"!==this.localName||"input"===this.localName&&"reset"!==this.type&&"image"!==this.type&&"button"!==this.type||this.disabled||this.form&&"submit"===this.type)return null;let t=i.get(this);if(t&&t.isConnected)return t;if(t&&!t.isConnected)return i.delete(this),null;let e=h(this),o=this.getAttribute("popovertarget");return(e instanceof Document||e instanceof L)&&o&&e.getElementById(o)||null}},popoverTargetAction:{enumerable:!0,configurable:!0,get(){let t=(this.getAttribute("popovertargetaction")||"").toLowerCase();return"show"===t||"hide"===t?t:"toggle"},set(t){this.setAttribute("popovertargetaction",t)}}})}s(HTMLButtonElement),s(HTMLInputElement);(e=document).addEventListener("click",t=>{let e=t.composedPath(),o=e[0];if(!(o instanceof Element)||o?.shadowRoot)return;let n=h(o);if(!(n instanceof L||n instanceof Document))return;let r=e.find(t=>t.matches?.("[popovertargetaction],[popovertarget]"));if(r){!function(t){let e=t.popoverTargetElement;if(!(e instanceof HTMLElement))return;let o=l(e);"show"===t.popoverTargetAction&&"showing"===o||("hide"!==t.popoverTargetAction||"hidden"!==o)&&("showing"===o?g(e,!0,!0):c(e,!1)&&(p.set(e,t),v(e)))}(r),t.preventDefault();return}}),e.addEventListener("keydown",t=>{let e=t.key,o=t.target;!t.defaultPrevented&&o&&("Escape"===e||"Esc"===e)&&b(o.ownerDocument,!0,!0)}),e.addEventListener("pointerdown",E),e.addEventListener("pointerup",E),x(document)}()}};
//# sourceMappingURL=t2-19741658de225445-4857ebe08d348da6.js.map