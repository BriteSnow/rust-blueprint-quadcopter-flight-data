
# Coding Rules

Always folow the rust10x best practices, and always include the rust10x best-practices-general.md

## UI Rules

- For selected tab or segmented control indicators, use a flat bottom indicator: `box-shadow: inset 0 -2px 0 var(--accent)` with `border-radius: 0` on the tab, matching `.speed-tab`.
- Do not round the bottom corners of a selected tab, since an inset shadow follows the border radius and makes the indicator curve.
