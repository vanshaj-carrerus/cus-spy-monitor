import mongoose from "mongoose";

const ManagerSchema = new mongoose.Schema({
    userId: { type: mongoose.Schema.Types.ObjectId, ref: 'User', required: true, unique: true },
    managedDepartments: [{ type: mongoose.Schema.Types.ObjectId, ref: 'Department' }],
    permissions: [String],
}, { timestamps: true });

const Manager = mongoose.models.Manager || mongoose.model("Manager", ManagerSchema);
export default Manager;
