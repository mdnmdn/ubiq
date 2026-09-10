```
AskUserQuestion({
  questions: Array<{
    question: string           // The complete question (ends with ?)
    header: string             // Short label (max 12 chars) displayed as chip
    options: Array<{
      label: string            // Display text (1-5 words)
      description: string      // Explanation of what this option means
      preview?: string         // Optional preview content (code snippets, mockups, etc.)
    }>                         // Must have 2-4 options
    multiSelect: boolean       // true = multiple selections, false = single choice
  }>,

  // Optional:
  annotations?: Record<string, {
    notes?: string             // Free-text notes user added to selection
    preview?: string           // Preview content of selected option
  }>,

  metadata?: {
    source?: string            // Optional identifier for tracking (e.g., "remember")
  }
})

Constraints:
- 1-4 questions per call
- Each question must have 2-4 options
- Header max 12 characters
- Labels should be 1-5 words
- Previews are only supported for single-select questions (not multiSelect)
- Users can always select "Other" to provide custom text input
```