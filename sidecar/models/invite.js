import mongoose from "mongoose";

const InviteSchema = new mongoose.Schema({
    email: { type: String, required: true, unique: true },
    role: { type: String, enum: ['admin', 'manager', 'marketing', 'sales'], required: true },
    token: { type: String, required: true },
    invitedBy: { type: mongoose.Schema.Types.ObjectId, ref: 'User' },
    status: { type: String, enum: ['pending', 'accepted', 'expired'], default: 'pending' },
    expiresAt: { type: Date, default: () => new Date(+new Date() + 7*24*60*60*1000) } // 7 days from now
}, { timestamps: true });

const Invite = mongoose.models.Invite || mongoose.model("Invite", InviteSchema);
export default Invite;
