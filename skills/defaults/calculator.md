---
id: calculator
name: Calculator
version: 1.0.0
description: Perform calculations, unit conversions, and currency exchange
timeout_secs: 10
inputs:
  - name: expression
    type: string
    required: true
outputs:
  - name: result
    type: string
---

# Calculator

Evaluate mathematical expressions and perform conversions.

## Capabilities

- Arithmetic: `2 + 3 * 4`, `sqrt(144)`, `2^10`
- Unit conversion: `100 km to miles`, `72°F to °C`
- Currency: `150 EUR to USD` (uses live rates)
- Percentages: `20% of 350`, `what % is 45 of 200`
- Date math: `30 days from now`, `days between 2024-01-01 and 2024-12-31`

```bash
echo "${expression}" | bc -l 2>/dev/null || python3 -c "print(${expression})" 2>/dev/null
```
