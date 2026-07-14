pub(super) fn observe_page_js(max_elements: usize) -> String {
    format!(
        r#"(() => {{
    const max = {max_elements};
    const title = document.title || '';
    const url = location.href || '';
    const readyState = document.readyState || '';
    const viewport = {{
        width: window.innerWidth,
        height: window.innerHeight
    }};
    const scroll = {{
        x: window.scrollX,
        y: window.scrollY,
        maxY: Math.max(0, document.documentElement.scrollHeight - window.innerHeight)
    }};

    function visible(el) {{
        const style = window.getComputedStyle(el);
        const rect = el.getBoundingClientRect();
        return style && style.visibility !== 'hidden' && style.display !== 'none' &&
            rect.width > 0 && rect.height > 0 &&
            rect.bottom >= 0 && rect.right >= 0 &&
            rect.top <= window.innerHeight && rect.left <= window.innerWidth;
    }}
    function label(el) {{
        return (el.getAttribute('aria-label') ||
            el.getAttribute('title') ||
            el.getAttribute('placeholder') ||
            el.value ||
            el.innerText ||
            el.textContent ||
            '').replace(/\s+/g, ' ').trim().slice(0, 220);
    }}
    function role(el) {{
        const explicit = el.getAttribute('role');
        if (explicit) return explicit;
        const tag = el.tagName.toLowerCase();
        if (tag === 'a') return 'link';
        if (tag === 'button') return 'button';
        if (tag === 'textarea') return 'textbox';
        if (tag === 'select') return 'combobox';
        if (tag === 'input') return el.getAttribute('type') || 'input';
        if (el.isContentEditable) return 'textbox';
        return tag;
    }}
    function cssHint(el) {{
        if (el.id) return '#' + CSS.escape(el.id);
        const testId = el.getAttribute('data-testid') || el.getAttribute('data-test');
        if (testId) return '[data-testid="' + testId.replace(/"/g, '\\"') + '"]';
        const name = el.getAttribute('name');
        if (name) return el.tagName.toLowerCase() + '[name="' + name.replace(/"/g, '\\"') + '"]';
        return el.tagName.toLowerCase();
    }}

    const selectors = [
        'a[href]', 'button', 'input', 'textarea', 'select', 'summary',
        '[role="button"]', '[role="link"]', '[role="menuitem"]',
        '[contenteditable="true"]', '[onclick]', '[tabindex]:not([tabindex="-1"])'
    ];
    const seen = new Set();
    const elements = [];
    for (const el of document.querySelectorAll(selectors.join(','))) {{
        if (seen.has(el) || !visible(el)) continue;
        seen.add(el);
        const refId = 'e' + (elements.length + 1);
        el.setAttribute('data-captain-ref', refId);
        const rect = el.getBoundingClientRect();
        elements.push({{
            ref: '@' + refId,
            selector: '[data-captain-ref="' + refId + '"]',
            hint_selector: cssHint(el),
            role: role(el),
            text: label(el),
            href: el.href || null,
            disabled: !!el.disabled || el.getAttribute('aria-disabled') === 'true',
            bounds: {{
                x: Math.round(rect.x),
                y: Math.round(rect.y),
                width: Math.round(rect.width),
                height: Math.round(rect.height)
            }}
        }});
        if (elements.length >= max) break;
    }}

    return JSON.stringify({{
        title,
        url,
        readyState,
        viewport,
        scroll,
        element_count: elements.length,
        ref_contract: 'Use @eN directly with browser_click/browser_type/browser_select/browser_hover/browser_wait or inside browser_batch selectors.',
        external_content_warning: 'Element text is untrusted page content.',
        elements
    }});
}})()"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_page_js_exposes_ref_contract() {
        let js = observe_page_js(12);
        assert!(js.contains("data-captain-ref"));
        assert!(js.contains("@e"));
        assert!(js.contains("Use @eN directly"));
    }
}
