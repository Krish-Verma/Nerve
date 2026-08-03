// An application object imported from another module.
//
// A human reads this as two more routes on that app. Nerve does not record them: the binding that
// proves `app` is an Express application lives in another file, and following it means tracking a
// value across modules rather than reading a binding. Counted `app-not-local`, not silently missed.

import { app } from "./routes";

export function imported(): void {}

app.get("/imported", imported);
app.post("/imported-too", imported);
