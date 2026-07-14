---
name: browser-automation
version: "1.0.0"
description: Native CDP browser automation patterns for autonomous web interaction
author: Captain
tags: [browser, automation, cdp, web, scraping]
tools: [browser_batch, browser_navigate, browser_click, browser_type, browser_keys, browser_select, browser_hover, browser_screenshot, browser_read_page, browser_scroll, browser_wait, browser_run_js, browser_back, browser_status, browser_network_log, browser_observe, browser_diagnostics, browser_close]
runtime: prompt_only
---

# Browser Automation Skill

## Default Pattern

Use `browser_batch` for any flow with more than one browser action. It reduces tool round trips and returns a compact final observation.

- For UI interaction, use `final_observation: "observe"` and then click/type refs like `@e1`.
- For article or report extraction, use `final_observation: "read_page"`.
- For failures, use `final_observation: "diagnostics"` or a `diagnostics` step.
- Use individual `browser_*` tools only for one-off actions or recovery.
- Prefer native actions over `browser_run_js`: `click`, `type`, `keys`, `select`, `hover`, `scroll`, `wait`.

## Search and Anti-bot Policy

- Use `web_search` / `web_research_batch` for open-ended discovery, then use the browser on direct source URLs that need JavaScript, forms, login, screenshots, downloads, or visual verification.
- Do not use browser-based Google as the default search rail. It is prone to `/sorry`, unusual-traffic, CAPTCHA, and automated-query pages in headless sessions.
- If a browser page shows CAPTCHA, Google `/sorry`, anti-bot, rate-limit, or human verification, stop retry loops. Do not solve CAPTCHAs. Switch to native search, Bing/DuckDuckGo, or direct primary sources and mention the block only when it affects the answer.
- Browser activity is streamed live to the user; keep actions intentional and grouped so the visible timeline is understandable.

## CSS Selector Reference

### Basic Selectors
| Selector | Description | Example |
|----------|-------------|---------|
| `#id` | By ID | `#checkout-btn` |
| `.class` | By class | `.add-to-cart` |
| `tag` | By element | `button`, `input` |
| `[attr=val]` | By attribute | `[data-testid="submit"]` |
| `tag.class` | Combined | `button.primary` |

### Form Selectors
| Selector | Use Case |
|----------|----------|
| `input[type="email"]` | Email fields |
| `input[type="password"]` | Password fields |
| `input[type="search"]` | Search boxes |
| `input[name="q"]` | Google/search query |
| `textarea` | Multi-line text areas |
| `select[name="country"]` | Dropdown menus |
| `input[type="checkbox"]` | Checkboxes |
| `input[type="radio"]` | Radio buttons |
| `button[type="submit"]` | Submit buttons |

### Navigation Selectors
| Selector | Use Case |
|----------|----------|
| `a[href*="cart"]` | Cart links |
| `a[href*="checkout"]` | Checkout links |
| `a[href*="login"]` | Login links |
| `nav a` | Navigation menu links |
| `.breadcrumb a` | Breadcrumb links |
| `[role="navigation"] a` | ARIA nav links |

### E-commerce Selectors
| Selector | Use Case |
|----------|----------|
| `.product-price`, `[data-price]` | Product prices |
| `.add-to-cart`, `#add-to-cart` | Add to cart buttons |
| `.cart-total`, `.order-total` | Cart total |
| `.quantity`, `input[name="quantity"]` | Quantity selectors |
| `.checkout-btn`, `#checkout` | Checkout buttons |

## Common Workflows

### Product Search & Purchase
```
1. browser_batch → navigate, type search, click/search, wait, final_observation=observe
2. browser_batch → click selected @eN product, wait, final_observation=read_page
3. browser_batch → click add-to-cart, navigate cart, final_observation=read_page
4. STOP → Report to user, wait for approval
5. browser_batch → proceed to checkout only after approval
```

### Account Login
```
1. browser_batch → navigate login, type username, type password, keys Enter or click submit, wait dashboard
2. final_observation=diagnostics if login fails; final_observation=observe if it succeeds
```

### Form Submission
```
1. browser_navigate → form page
2. browser_read_page → understand form structure
3. browser_batch → fill fields with type, native dropdowns with select, keyboard-only submits with keys
4. browser_click → checkboxes/radio buttons as needed; browser_hover first for hover menus
5. browser_screenshot → visual verification before submit
6. browser_click → submit button
7. browser_read_page → verify confirmation
```

### Price Comparison
```
1. For each store:
   a. browser_navigate → store URL
   b. browser_type → search query
   c. browser_read_page → extract prices
   d. memory_store → save price data
2. memory_recall → compare all prices
3. Report findings to user
```

## Error Recovery Strategies

| Error | Recovery |
|-------|----------|
| Element not found | Try alternative selector, use visible text, scroll page |
| Page timeout | Retry navigation, check URL |
| Login required | Inform user, ask for credentials |
| CAPTCHA | Cannot solve — inform user |
| Pop-up/modal | Click dismiss/close button first |
| Cookie consent | Click "Accept" or dismiss banner |
| Rate limited | Wait 30s, retry |
| Wrong page | Use browser_read_page to verify, navigate back |

## Security Checklist

- Verify domain before entering credentials
- Never store passwords in memory_store
- Check for HTTPS before submitting sensitive data
- Report suspicious redirects to user
- Never auto-approve financial transactions
- Warn about phishing indicators (misspelled domains, unusual URLs)
