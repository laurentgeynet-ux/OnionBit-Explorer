import{E as t,T as s,P as e,a,i as n,x as r,d as o}from"./vaadin-button.js";import{renderMarkdownToElement as i}from"./markdown-helpers.js";class d extends(t(s(e(a)))){static get is(){return"vaadin-markdown"}static get styles(){return n`
      :host {
        display: block;
      }

      :host([hidden]) {
        display: none !important;
      }
    `}static get properties(){return{content:{type:String,sync:!0}}}render(){return r`<slot></slot>`}updated(t){super.updated(t),t.has("content")&&i(this,this.content)}}o(d);export{d as Markdown};
