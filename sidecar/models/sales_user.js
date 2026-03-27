import mongoose from "mongoose";

const SalesUserSchema = new mongoose.Schema({
    userId: { type: mongoose.Schema.Types.ObjectId, ref: 'User', required: true, unique: true },
    departmentId: { type: mongoose.Schema.Types.ObjectId, ref: 'Department' },
    locationId: { type: mongoose.Schema.Types.ObjectId, ref: 'Location' },
    managerId: { type: mongoose.Schema.Types.ObjectId, ref: 'Manager' },
    active: { type: Boolean, default: false },
    lastLogin: { type: Date, default: Date.now }
}, { timestamps: true });

const SalesUser = mongoose.models.SalesUser || mongoose.model("SalesUser", SalesUserSchema);
export default SalesUser;