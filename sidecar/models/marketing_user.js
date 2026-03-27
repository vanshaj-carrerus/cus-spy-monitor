import mongoose from "mongoose";

const MarketingUserSchema = new mongoose.Schema({
    userId: { type: mongoose.Schema.Types.ObjectId, ref: 'User', required: true, unique: true },
    departmentId: { type: mongoose.Schema.Types.ObjectId, ref: 'Department' },
    locationId: { type: mongoose.Schema.Types.ObjectId, ref: 'Location' },
    managerId: { type: mongoose.Schema.Types.ObjectId, ref: 'Manager' },
    active: { type: Boolean, default: false },
    lastLogin: { type: Date, default: Date.now }
}, { timestamps: true });

const MarketingUser = mongoose.models.MarketingUser || mongoose.model("MarketingUser", MarketingUserSchema);
export default MarketingUser;
