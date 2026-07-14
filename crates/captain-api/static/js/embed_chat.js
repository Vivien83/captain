(function() {
  'use strict';

  var script = document.currentScript || (function() {
    var scripts = document.getElementsByTagName('script');
    return scripts[scripts.length - 1];
  })();
  var scriptUrl = new URL(script && script.src || '/embed/chat.js', document.baseURI);
  var origin = scriptUrl.origin;
  var session = script && script.getAttribute('data-session') || 'main';
  var label = script && script.getAttribute('data-label') || 'Captain';
  var width = script && script.getAttribute('data-width') || '420px';
  var height = script && script.getAttribute('data-height') || '680px';

  if (document.getElementById('captain-embed-root')) return;

  var root = document.createElement('div');
  root.id = 'captain-embed-root';
  root.style.position = 'fixed';
  root.style.right = '18px';
  root.style.bottom = '18px';
  root.style.zIndex = '2147483000';
  root.style.fontFamily = 'Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif';

  var frame = document.createElement('iframe');
  frame.title = 'Captain';
  frame.src = origin + '/terminal?session=' + encodeURIComponent(session);
  frame.allow = 'microphone; clipboard-read; clipboard-write';
  frame.style.width = 'min(' + width + ', calc(100vw - 24px))';
  frame.style.height = 'min(' + height + ', calc(100vh - 88px))';
  frame.style.border = '1px solid rgba(191,253,0,.28)';
  frame.style.background = '#0a0a0a';
  frame.style.boxShadow = '0 24px 80px rgba(0,0,0,.72)';
  frame.style.display = 'none';

  var button = document.createElement('button');
  button.type = 'button';
  button.textContent = label;
  button.setAttribute('aria-expanded', 'false');
  button.style.height = '42px';
  button.style.padding = '0 16px';
  button.style.border = '1px solid #1a1a00';
  button.style.borderRadius = '0';
  button.style.background = '#bffd00';
  button.style.color = '#0a0a0a';
  button.style.font = '700 13px "IBM Plex Mono", "SF Mono", Menlo, monospace';
  button.style.cursor = 'pointer';
  button.style.float = 'right';

  button.addEventListener('click', function() {
    var open = frame.style.display === 'none';
    frame.style.display = open ? 'block' : 'none';
    button.setAttribute('aria-expanded', open ? 'true' : 'false');
    button.textContent = open ? 'Close' : label;
  });

  root.appendChild(frame);
  root.appendChild(button);
  document.body.appendChild(root);
})();
