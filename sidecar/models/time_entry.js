import mongoose from "mongoose";

const TimeEntrySchema = new mongoose.Schema({
    userId: { type: String, required: true },
    date: { type: Date, required: true },
    activeSeconds: { type: Number, default: 0 },
}, { timestamps: true });

const TimeEntry = mongoose.models.TimeEntry || mongoose.model("TimeEntry", TimeEntrySchema);
export default TimeEntry;
