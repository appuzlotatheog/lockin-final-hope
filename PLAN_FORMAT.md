# Lockin Plan Format

Version `1`. Files are plain JSON, UTF-8, maximum 5 MB for plan import and
10 MB for full backups.

## Top-level shape

```json
{
  "format": "lockin-plan",
  "schema_version": 1,
  "plan": {
    "id": "kerala-plusone-christmas-2026-abc123",
    "name": "Kerala Plus One — Christmas 2026",
    "timezone": "Asia/Kolkata",
    "start_date": "2026-11-16",
    "end_date": "2026-12-17",
    "academic_year": "2026"
  },
  "subjects": [
    {
      "id": "sub-physics",
      "name": "Physics",
      "color": "#9B7BFF",
      "sort_order": 0,
      "chapters": [
        { "id": "ch-physics-1", "title": "Physical World", "order": 0, "status": "not_started" }
      ]
    }
  ],
  "exams": [
    { "id": "exam-physics", "subject_id": "sub-physics", "label": "Physics", "date": "2026-11-30", "start_time": "13:30", "session": "Afternoon" }
  ],
  "schedule_templates": [],
  "tasks": [],
  "revision_policy": { "interval_days": [1, 3, 7], "adapt_to_results": true }
}
```

## Rules

- `format` must equal `lockin-plan`.
- `schema_version` must equal `1`.
- Dates are ISO `YYYY-MM-DD`; times are `HH:MM`.
- `timezone` must be a valid IANA name.
- Subject and chapter IDs must be unique within the file; exams and slots must
  reference existing subjects.
- Duplicate or overlapping template slots produce warnings, not errors.
- Import in **merge** mode updates the existing plan in place and preserves
  completed tasks, sessions, tests, sleep entries and revision history.
- Import in **replace** mode creates a new plan and never deletes historical
  records from other plans.
- Backups are full database snapshots in the same JSON envelope with a
  `backup` section; restore validates before replacing local data.

## Schema

`sząemas/lockin-plan.schema.json` (JSON Schema draft 2020-12). See
`examples/christmas-2026-plan.json` for a complete seeded example.
