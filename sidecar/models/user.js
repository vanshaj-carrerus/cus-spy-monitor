import mongoose from "mongoose";

const UserSchema = new mongoose.Schema({
    username: { type: String, required: true, unique: true },
    password: { type: String, required: true },
    email: { type: String, required: true, unique: true },
    role: { type: String, enum: ['admin', 'manager', 'marketing', 'sales'], default: 'sales' },
}, { timestamps: true });

const User = mongoose.models.User || mongoose.model("User", UserSchema);
export default User;
