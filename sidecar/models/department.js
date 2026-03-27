import mongoose from "mongoose";

const DepartmentSchema = new mongoose.Schema({
    name: { type: String, required: true, unique: true },
    description: { type: String },
}, { timestamps: true });

const Department = mongoose.models.Department || mongoose.model("Department", DepartmentSchema);
export default Department;
